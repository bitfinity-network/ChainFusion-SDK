#![allow(dead_code)]

use crate::context::{CanisterType, TestContext};
use crate::state_machine_tests::StateMachineContext;
use crate::utils::wasm::{
    get_btc_bridge_canister_bytecode, get_btc_canister_bytecode,
    get_ck_btc_minter_canister_bytecode, get_icrc1_token_canister_bytecode,
    get_kyt_canister_bytecode,
};
use bitcoin::util::psbt::serialize::Deserialize;
use bitcoin::{Address as BtcAddress, Network as BtcNetwork};
use btc_bridge::canister::eth_address_to_subaccount;
use btc_bridge::ck_btc_interface::PendingUtxo;
use btc_bridge::interface::{Erc20MintError, Erc20MintStatus};
use btc_bridge::state::{BftBridgeConfig, BtcBridgeConfig};
use candid::{Decode, Encode, Nat, Principal};
use did::H160;
use eth_signer::sign_strategy::SigningStrategy;
use eth_signer::Signer;
use ic_base_types::{CanisterId, PrincipalId};
use ic_bitcoin_canister_mock::{OutPoint, PushUtxoToAddress, Utxo};
use ic_btc_interface::{Network, Txid};
use ic_canister_client::CanisterClient;
use ic_canisters_http_types::{HttpRequest, HttpResponse};
use ic_ckbtc_kyt::{InitArg as KytInitArg, KytMode, LifecycleArg, SetApiKeyArg};
use ic_ckbtc_minter::lifecycle::init::{InitArgs as CkbtcMinterInitArgs, MinterArg};
use ic_ckbtc_minter::lifecycle::upgrade::UpgradeArgs;
use ic_ckbtc_minter::queries::{EstimateFeeArg, RetrieveBtcStatusRequest, WithdrawalFee};
use ic_ckbtc_minter::state::{BtcRetrievalStatusV2, Mode, RetrieveBtcStatus, RetrieveBtcStatusV2};
use ic_ckbtc_minter::updates::get_btc_address::GetBtcAddressArgs;
use ic_ckbtc_minter::updates::retrieve_btc::{
    RetrieveBtcArgs, RetrieveBtcError, RetrieveBtcOk, RetrieveBtcWithApprovalArgs,
    RetrieveBtcWithApprovalError,
};
use ic_ckbtc_minter::updates::update_balance::{UpdateBalanceArgs, UpdateBalanceError, UtxoStatus};
use ic_ckbtc_minter::{Log, MinterInfo, CKBTC_LEDGER_MEMO_SIZE, MIN_RELAY_FEE_PER_VBYTE};
use ic_exports::ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;
use ic_exports::icrc_types::icrc1::account::Account;
use ic_exports::icrc_types::icrc1::transfer::{TransferArg, TransferError};
use ic_exports::icrc_types::icrc2::approve::{ApproveArgs, ApproveError};
use ic_exports::icrc_types::icrc3::transactions::{
    GetTransactionsRequest, GetTransactionsResponse,
};
use ic_icrc1_ledger::{InitArgsBuilder as LedgerInitArgsBuilder, LedgerArgument};
use ic_state_machine_tests::{Cycles, StateMachine, StateMachineBuilder, WasmResult};
use minter_contract_utils::evm_link::EvmLink;
use minter_did::id256::Id256;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

const KYT_FEE: u64 = 2_000;
const CKBTC_LEDGER_FEE: u64 = 10;
const TRANSFER_FEE: u64 = 10;
const MIN_CONFIRMATIONS: u32 = 12;
const MAX_TIME_IN_QUEUE: Duration = Duration::from_secs(10);
const WITHDRAWAL_ADDRESS: &str = "bc1q34aq5drpuwy3wgl9lhup9892qp6svr8ldzyy7c";

fn ledger_wasm() -> Vec<u8> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(get_icrc1_token_canister_bytecode())
}

fn minter_wasm() -> Vec<u8> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(get_ck_btc_minter_canister_bytecode())
}

fn bitcoin_mock_wasm() -> Vec<u8> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(get_btc_canister_bytecode())
}

fn kyt_wasm() -> Vec<u8> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(get_kyt_canister_bytecode())
}

fn install_ledger(env: &StateMachine) -> CanisterId {
    let args = LedgerArgument::Init(
        LedgerInitArgsBuilder::for_tests()
            .with_transfer_fee(0_u8)
            .build(),
    );
    env.install_canister(ledger_wasm(), Encode!(&args).unwrap(), None)
        .unwrap()
}

fn install_minter(env: &StateMachine, ledger_id: CanisterId) -> CanisterId {
    let args = CkbtcMinterInitArgs {
        btc_network: ic_ckbtc_minter::lifecycle::init::BtcNetwork::Regtest,
        // The name of the [EcdsaKeyId]. Use "dfx_test_key" for local replica and "test_key_1" for
        // a testing key for testnet and mainnet
        ecdsa_key_name: "dfx_test_key".parse().unwrap(),
        retrieve_btc_min_amount: 2000,
        ledger_id,
        max_time_in_queue_nanos: 0,
        min_confirmations: Some(1),
        mode: Mode::GeneralAvailability,
        kyt_fee: None,
        kyt_principal: Some(CanisterId::from(0)),
    };
    let minter_arg = MinterArg::Init(args);
    env.install_canister(minter_wasm(), Encode!(&minter_arg).unwrap(), None)
        .unwrap()
}

fn assert_reply(result: WasmResult) -> Vec<u8> {
    match result {
        WasmResult::Reply(bytes) => bytes,
        WasmResult::Reject(reject) => {
            panic!("Expected a successful reply, got a reject: {}", reject)
        }
    }
}

fn input_utxos(tx: &bitcoin::Transaction) -> Vec<bitcoin::OutPoint> {
    tx.input.iter().map(|txin| txin.previous_output).collect()
}

fn assert_replacement_transaction(old: &bitcoin::Transaction, new: &bitcoin::Transaction) {
    assert_ne!(old.txid(), new.txid());
    assert_eq!(input_utxos(old), input_utxos(new));

    let new_out_value = new.output.iter().map(|out| out.value).sum::<u64>();
    let prev_out_value = old.output.iter().map(|out| out.value).sum::<u64>();
    let relay_cost = new.vsize() as u64 * MIN_RELAY_FEE_PER_VBYTE / 1000;

    assert!(
        new_out_value + relay_cost <= prev_out_value,
        "the transaction fees should have increased by at least {relay_cost}. prev out value: {prev_out_value}, new out value: {new_out_value}"
    );
}

fn vec_to_txid(vec: Vec<u8>) -> [u8; 32] {
    let bytes: [u8; 32] = vec.try_into().expect("Vector length must be exactly 32");
    bytes
}

fn range_to_txid(range: std::ops::RangeInclusive<u8>) -> [u8; 32] {
    vec_to_txid(range.collect::<Vec<u8>>())
}

#[test]
fn test_install_ckbtc_minter_canister() {
    let env = StateMachine::new();
    let ledger_id = install_ledger(&env);
    install_minter(&env, ledger_id);
}

fn mainnet_bitcoin_canister_id() -> CanisterId {
    CanisterId::try_from(
        PrincipalId::from_str(ic_config::execution_environment::BITCOIN_MAINNET_CANISTER_ID)
            .unwrap(),
    )
    .unwrap()
}

fn install_bitcoin_mock_canister(env: &StateMachine) {
    let args = Network::Mainnet;
    let cid = mainnet_bitcoin_canister_id();
    env.create_canister_with_cycles(Some(cid.into()), Cycles::new(0), None);

    env.install_existing_canister(cid, bitcoin_mock_wasm(), Encode!(&args).unwrap())
        .unwrap();
}

struct CkBtcSetup {
    pub context: StateMachineContext,
    pub caller: PrincipalId,
    pub kyt_provider: PrincipalId,
    pub bitcoin_id: CanisterId,
    pub ledger_id: CanisterId,
    pub minter_id: CanisterId,
    pub kyt_id: CanisterId,
    pub tip_height: AtomicU32,
    pub wrapped_token: H160,
}

impl CkBtcSetup {
    pub async fn new() -> Self {
        let bitcoin_id = mainnet_bitcoin_canister_id();
        let caller = PrincipalId::new_user_test_id(1);

        let (env, ledger_id, minter_id, kyt_id, kyt_provider) =
            tokio::task::spawn_blocking(move || {
                let env = StateMachineBuilder::new()
                    .with_default_canister_range()
                    .with_extra_canister_range(bitcoin_id..=bitcoin_id)
                    .build();

                install_bitcoin_mock_canister(&env);
                let ledger_id = env.create_canister(None);
                let minter_id =
                    env.create_canister_with_cycles(None, Cycles::new(100_000_000_000_000), None);
                let kyt_id = env.create_canister(None);

                env.install_existing_canister(
                    ledger_id,
                    ledger_wasm(),
                    Encode!(&LedgerArgument::Init(
                        LedgerInitArgsBuilder::with_symbol_and_name("ckBTC", "ckBTC")
                            .with_minting_account(minter_id.get().0)
                            .with_transfer_fee(TRANSFER_FEE)
                            .with_max_memo_length(CKBTC_LEDGER_MEMO_SIZE)
                            .with_feature_flags(ic_icrc1_ledger::FeatureFlags { icrc2: true })
                            .build()
                    ))
                    .unwrap(),
                )
                .expect("failed to install the ledger");

                env.install_existing_canister(
                    minter_id,
                    minter_wasm(),
                    Encode!(&MinterArg::Init(CkbtcMinterInitArgs {
                        btc_network: ic_ckbtc_minter::lifecycle::init::BtcNetwork::Mainnet,
                        ecdsa_key_name: "master_ecdsa_public_key".to_string(),
                        retrieve_btc_min_amount: 100_000,
                        ledger_id,
                        max_time_in_queue_nanos: 100,
                        min_confirmations: Some(MIN_CONFIRMATIONS),
                        mode: Mode::GeneralAvailability,
                        kyt_fee: Some(KYT_FEE),
                        kyt_principal: kyt_id.into(),
                    }))
                    .unwrap(),
                )
                .expect("failed to install the minter");

                let kyt_provider = PrincipalId::new_user_test_id(2);

                env.install_existing_canister(
                    kyt_id,
                    kyt_wasm(),
                    Encode!(&LifecycleArg::InitArg(KytInitArg {
                        minter_id: minter_id.into(),
                        maintainers: vec![kyt_provider.into()],
                        mode: KytMode::AcceptAll,
                    }))
                    .unwrap(),
                )
                .expect("failed to install the KYT canister");

                env.execute_ingress(
                    bitcoin_id,
                    "set_fee_percentiles",
                    Encode!(&(1..=100).map(|i| i * 100).collect::<Vec<u64>>()).unwrap(),
                )
                .expect("failed to set fee percentiles");

                env.execute_ingress_as(
                    kyt_provider,
                    kyt_id,
                    "set_api_key",
                    Encode!(&SetApiKeyArg {
                        api_key: "api key".to_string(),
                    })
                    .unwrap(),
                )
                .expect("failed to set api key");

                (env, ledger_id, minter_id, kyt_id, kyt_provider)
            })
            .await
            .unwrap();

        let mut context = StateMachineContext::new(env);
        context.canisters.set(CanisterType::Kyt, kyt_id.into());
        context
            .canisters
            .set(CanisterType::CkBtcMinter, minter_id.into());

        let canisters = [
            CanisterType::Signature,
            CanisterType::Evm,
            CanisterType::Minter,
        ];
        for canister_type in canisters {
            context.canisters.set(
                canister_type,
                (&context)
                    .create_canister()
                    .await
                    .expect("failed to create a canister"),
            );
        }

        for canister_type in canisters {
            (&context).install_default_canister(canister_type).await
        }

        let wallet = (&context).new_wallet(u128::MAX).await.unwrap();

        let config = BtcBridgeConfig {
            ck_btc_minter: minter_id.into(),
            ck_btc_ledger: ledger_id.into(),
            network: BitcoinNetwork::Mainnet,
            evm_link: EvmLink::Ic((&context).canisters().evm()),
            signing_strategy: SigningStrategy::Local {
                private_key: [2; 32],
            },
            admin: (&context).admin(),
            ck_btc_ledger_fee: CKBTC_LEDGER_FEE,
        };

        let btc_bridge = (&context).create_canister().await.unwrap();
        (&context)
            .install_canister(
                btc_bridge,
                get_btc_bridge_canister_bytecode().await,
                (config,),
            )
            .await
            .unwrap();
        context.canisters.set(CanisterType::BtcBridge, btc_bridge);

        let btc_bridge_eth_address: Option<H160> = (&context)
            .client(btc_bridge, "admin")
            .update("get_evm_address", ())
            .await
            .unwrap();

        let client = (&context).evm_client("admin");
        client
            .mint_native_tokens(btc_bridge_eth_address.clone().unwrap(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();

        let bft_bridge = (&context)
            .initialize_bft_bridge_with_minter(&wallet, btc_bridge_eth_address.unwrap())
            .await
            .unwrap();
        let token_id = Id256::from(&Principal::from(ledger_id));
        let token = (&context)
            .create_wrapped_token(&wallet, &bft_bridge, token_id)
            .await
            .unwrap();

        let chain_id = (&context).evm_client("admin").eth_chain_id().await.unwrap();

        let mut token_name = [0; 32];
        token_name[0..7].copy_from_slice(b"wrapper");
        let mut token_symbol = [0; 16];
        token_symbol[0..3].copy_from_slice(b"WPT");

        let bft_config = BftBridgeConfig {
            erc20_chain_id: chain_id as u32,
            bridge_address: bft_bridge,
            token_address: token.clone(),
            token_name,
            token_symbol,
            decimals: 0,
        };

        let _: () = (&context)
            .client(btc_bridge, "admin")
            .update("admin_configure_bft_bridge", (bft_config,))
            .await
            .unwrap();

        (&context).advance_time(Duration::from_secs(2)).await;

        Self {
            context,
            kyt_provider,
            caller,
            bitcoin_id,
            ledger_id,
            minter_id,
            kyt_id,
            wrapped_token: token,
            tip_height: AtomicU32::default(),
        }
    }

    pub fn env(&self) -> Arc<StateMachine> {
        self.context.env.clone()
    }

    pub fn set_fee_percentiles(&self, fees: &Vec<u64>) {
        self.env()
            .execute_ingress(
                self.bitcoin_id,
                "set_fee_percentiles",
                Encode!(fees).unwrap(),
            )
            .expect("failed to set fee percentiles");
    }

    pub fn set_tip_height(&self, tip_height: u32) {
        self.tip_height.store(tip_height, Ordering::Relaxed);
        self.env()
            .execute_ingress(
                self.bitcoin_id,
                "set_tip_height",
                Encode!(&tip_height).unwrap(),
            )
            .expect("failed to set fee tip height");
    }

    pub fn advance_tip_height(&self, delta: u32) {
        let prev_value = self.tip_height.fetch_add(delta, Ordering::Relaxed);
        self.env()
            .execute_ingress(
                self.bitcoin_id,
                "set_tip_height",
                Encode!(&(prev_value + delta)).unwrap(),
            )
            .expect("failed to set fee tip height");
    }

    pub fn push_utxo(&self, address: String, utxo: Utxo) {
        assert_reply(
            self.env()
                .execute_ingress(
                    self.bitcoin_id,
                    "push_utxo_to_address",
                    Encode!(&PushUtxoToAddress { address, utxo }).unwrap(),
                )
                .expect("failed to push a UTXO"),
        );
    }

    pub fn get_btc_address(&self, account: impl Into<Account>) -> String {
        let account = account.into();
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress_as(
                        self.caller,
                        self.minter_id,
                        "get_btc_address",
                        Encode!(&GetBtcAddressArgs {
                            owner: Some(account.owner),
                            subaccount: account.subaccount,
                        })
                        .unwrap(),
                    )
                    .expect("failed to get btc address")
            ),
            String
        )
        .unwrap()
    }

    pub fn get_minter_info(&self) -> MinterInfo {
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress(self.minter_id, "get_minter_info", Encode!().unwrap(),)
                    .expect("failed to get minter info")
            ),
            MinterInfo
        )
        .unwrap()
    }

    pub fn get_logs(&self) -> Log {
        let request = HttpRequest {
            method: "".to_string(),
            url: "/logs".to_string(),
            headers: vec![],
            body: serde_bytes::ByteBuf::new(),
        };
        let response = Decode!(
            &assert_reply(
                self.env()
                    .query(self.minter_id, "http_request", Encode!(&request).unwrap(),)
                    .expect("failed to get minter info")
            ),
            HttpResponse
        )
        .unwrap();
        serde_json::from_slice(&response.body).expect("failed to parse ckbtc minter log")
    }

    pub fn refresh_fee_percentiles(&self) {
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress_as(
                        self.caller,
                        self.minter_id,
                        "refresh_fee_percentiles",
                        Encode!().unwrap()
                    )
                    .expect("failed to refresh fee percentiles")
            ),
            Option<Nat>
        )
        .unwrap();
    }

    pub fn estimate_withdrawal_fee(&self, amount: Option<u64>) -> WithdrawalFee {
        self.refresh_fee_percentiles();
        Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.minter_id,
                        "estimate_withdrawal_fee",
                        Encode!(&EstimateFeeArg { amount }).unwrap()
                    )
                    .expect("failed to query minter fee estimate")
            ),
            WithdrawalFee
        )
        .unwrap()
    }

    pub fn deposit_utxo(&self, account: impl Into<Account>, utxo: Utxo) {
        let account = account.into();
        let deposit_address = self.get_btc_address(account);

        self.push_utxo(deposit_address, utxo.clone());

        let utxo_status = Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress_as(
                        self.caller,
                        self.minter_id,
                        "update_balance",
                        Encode!(&UpdateBalanceArgs {
                            owner: Some(account.owner),
                            subaccount: account.subaccount,
                        })
                        .unwrap()
                    )
                    .expect("failed to update balance")
            ),
            Result<Vec<UtxoStatus>, UpdateBalanceError>
        )
        .unwrap();

        assert_eq!(
            utxo_status.unwrap(),
            vec![UtxoStatus::Minted {
                block_index: 0,
                minted_amount: utxo.value - KYT_FEE,
                utxo,
            }]
        );
    }

    pub fn get_transactions(&self, arg: GetTransactionsRequest) -> GetTransactionsResponse {
        Decode!(
            &assert_reply(
                self.env()
                    .query(self.ledger_id, "get_transactions", Encode!(&arg).unwrap())
                    .expect("failed to query get_transactions on the ledger")
            ),
            GetTransactionsResponse
        )
        .unwrap()
    }

    pub fn get_known_utxos(&self, account: impl Into<Account>) -> Vec<Utxo> {
        let account = account.into();
        Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.minter_id,
                        "get_known_utxos",
                        Encode!(&UpdateBalanceArgs {
                            owner: Some(account.owner),
                            subaccount: account.subaccount,
                        })
                        .unwrap()
                    )
                    .expect("failed to query balance on the ledger")
            ),
            Vec<Utxo>
        )
        .unwrap()
    }

    pub fn balance_of(&self, account: impl Into<Account>) -> Nat {
        Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.ledger_id,
                        "icrc1_balance_of",
                        Encode!(&account.into()).unwrap()
                    )
                    .expect("failed to query balance on the ledger")
            ),
            Nat
        )
        .unwrap()
    }

    pub fn withdrawal_account(&self, owner: PrincipalId) -> Account {
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress_as(
                        owner,
                        self.minter_id,
                        "get_withdrawal_account",
                        Encode!().unwrap()
                    )
                    .expect("failed to get ckbtc withdrawal account")
            ),
            Account
        )
        .unwrap()
    }

    pub fn transfer(&self, from: impl Into<Account>, to: impl Into<Account>, amount: u64) -> Nat {
        let from = from.into();
        let to = to.into();
        Decode!(&assert_reply(self.env().execute_ingress_as(
            PrincipalId::from(from.owner),
            self.ledger_id,
            "icrc1_transfer",
            Encode!(&TransferArg {
                from_subaccount: from.subaccount,
                to,
                fee: None,
                created_at_time: None,
                memo: None,
                amount: Nat::from(amount),
            }).unwrap()
            ).expect("failed to execute token transfer")),
            Result<Nat, TransferError>
        )
        .unwrap()
        .expect("token transfer failed")
    }

    pub fn approve_minter(
        &self,
        from: Principal,
        amount: u64,
        from_subaccount: Option<[u8; 32]>,
    ) -> Nat {
        Decode!(&assert_reply(self.env().execute_ingress_as(
            PrincipalId::from(from),
            self.ledger_id,
            "icrc2_approve",
            Encode!(&ApproveArgs {
                from_subaccount,
                spender: Account {
                    owner: self.minter_id.into(),
                    subaccount: None
                },
                amount: Nat::from(amount),
                expected_allowance: None,
                expires_at: None,
                fee: None,
                memo: None,
                created_at_time: None,
            }).unwrap()
            ).expect("failed to execute token transfer")),
            Result<Nat, ApproveError>
        )
        .unwrap()
        .expect("approve failed")
    }

    pub fn retrieve_btc(
        &self,
        address: String,
        amount: u64,
    ) -> Result<RetrieveBtcOk, RetrieveBtcError> {
        Decode!(
            &assert_reply(
                self.env().execute_ingress_as(self.caller, self.minter_id, "retrieve_btc", Encode!(&RetrieveBtcArgs {
                    address,
                    amount,
                }).unwrap())
                .expect("failed to execute retrieve_btc request")
            ),
            Result<RetrieveBtcOk, RetrieveBtcError>
        ).unwrap()
    }

    pub fn retrieve_btc_with_approval(
        &self,
        address: String,
        amount: u64,
        from_subaccount: Option<[u8; 32]>,
    ) -> Result<RetrieveBtcOk, RetrieveBtcWithApprovalError> {
        Decode!(
            &assert_reply(
                self.env().execute_ingress_as(self.caller, self.minter_id, "retrieve_btc_with_approval", Encode!(&RetrieveBtcWithApprovalArgs {
                    address,
                    amount,
                    from_subaccount
                }).unwrap())
                .expect("failed to execute retrieve_btc request")
            ),
            Result<RetrieveBtcOk, RetrieveBtcWithApprovalError>
        ).unwrap()
    }

    pub fn retrieve_btc_status(&self, block_index: u64) -> RetrieveBtcStatus {
        Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.minter_id,
                        "retrieve_btc_status",
                        Encode!(&RetrieveBtcStatusRequest { block_index }).unwrap()
                    )
                    .expect("failed to get ckbtc withdrawal account")
            ),
            RetrieveBtcStatus
        )
        .unwrap()
    }

    pub fn retrieve_btc_status_v2(&self, block_index: u64) -> RetrieveBtcStatusV2 {
        Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.minter_id,
                        "retrieve_btc_status_v2",
                        Encode!(&RetrieveBtcStatusRequest { block_index }).unwrap()
                    )
                    .expect("failed to retrieve_btc_status_v2")
            ),
            RetrieveBtcStatusV2
        )
        .unwrap()
    }

    pub fn retrieve_btc_status_v2_by_account(
        &self,
        maybe_account: Option<Account>,
    ) -> Vec<BtcRetrievalStatusV2> {
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress(
                        self.minter_id,
                        "retrieve_btc_status_v2_by_account",
                        Encode!(&maybe_account).unwrap()
                    )
                    .expect("failed to retrieve_btc_status_v2_by_account")
            ),
            Vec<BtcRetrievalStatusV2>
        )
        .unwrap()
    }

    pub fn tick_until<R>(
        &self,
        description: &str,
        max_ticks: u64,
        mut condition: impl FnMut(&CkBtcSetup) -> Option<R>,
    ) -> R {
        if let Some(result) = condition(self) {
            return result;
        }
        for _ in 0..max_ticks {
            self.env().tick();
            if let Some(result) = condition(self) {
                return result;
            }
        }
        self.print_minter_logs();
        self.print_minter_events();
        panic!(
            "did not reach condition '{}' in {} ticks",
            description, max_ticks
        )
    }

    /// Check that the given condition holds for the specified number of state machine ticks.
    pub fn assert_for_n_ticks(
        &self,
        description: &str,
        num_ticks: u64,
        mut condition: impl FnMut(&CkBtcSetup) -> bool,
    ) {
        for n in 0..num_ticks {
            self.env().tick();
            if !condition(self) {
                panic!(
                    "Condition '{}' does not hold after {} ticks",
                    description, n
                );
            }
        }
    }

    pub fn await_btc_transaction(&self, block_index: u64, max_ticks: usize) -> Txid {
        let mut last_status = None;
        for _ in 0..max_ticks {
            dbg!(self.get_logs());
            let status_v2 = self.retrieve_btc_status_v2(block_index);
            let status = self.retrieve_btc_status(block_index);
            assert_eq!(RetrieveBtcStatusV2::from(status.clone()), status_v2);
            match status {
                RetrieveBtcStatus::Submitted { txid } => {
                    return Txid::try_from(txid.as_ref()).unwrap();
                }
                status => {
                    last_status = Some(status);
                    self.env().tick();
                }
            }
        }
        panic!(
            "the minter did not submit a transaction in {} ticks; last status {:?}",
            max_ticks, last_status
        )
    }

    pub fn print_minter_events(&self) {
        use ic_ckbtc_minter::state::eventlog::{Event, GetEventsArg};
        let events = Decode!(
            &assert_reply(
                self.env()
                    .query(
                        self.minter_id,
                        "get_events",
                        Encode!(&GetEventsArg {
                            start: 0,
                            length: 2000,
                        })
                        .unwrap()
                    )
                    .expect("failed to query minter events")
            ),
            Vec<Event>
        )
        .unwrap();
        println!("{:#?}", events);
    }

    pub fn print_minter_logs(&self) {
        let log = self.get_logs();
        for entry in log.entries {
            println!(
                "{} {}:{} {}",
                entry.timestamp, entry.file, entry.line, entry.message
            );
        }
    }

    pub fn await_finalization(&self, block_index: u64, max_ticks: usize) -> Txid {
        let mut last_status = None;
        for _ in 0..max_ticks {
            let status_v2 = self.retrieve_btc_status_v2(block_index);
            let status = self.retrieve_btc_status(block_index);
            assert_eq!(RetrieveBtcStatusV2::from(status.clone()), status_v2);
            match status {
                RetrieveBtcStatus::Confirmed { txid } => {
                    return Txid::try_from(txid.as_ref()).unwrap();
                }
                status => {
                    last_status = Some(status);
                    self.env().tick();
                }
            }
        }
        panic!(
            "the minter did not finalize the transaction in {} ticks; last status: {:?}",
            max_ticks, last_status
        )
    }

    pub fn finalize_transaction(&self, tx: &bitcoin::Transaction) {
        let change_utxo = tx.output.last().unwrap();
        let change_address =
            BtcAddress::from_script(&change_utxo.script_pubkey, BtcNetwork::Bitcoin).unwrap();

        let main_address = self.get_btc_address(Principal::from(self.minter_id));
        assert_eq!(change_address.to_string(), main_address);

        self.env()
            .advance_time(MIN_CONFIRMATIONS * Duration::from_secs(600) + Duration::from_secs(1));
        let txid_bytes: [u8; 32] = tx.txid().to_vec().try_into().unwrap();
        self.push_utxo(
            change_address.to_string(),
            Utxo {
                value: change_utxo.value,
                height: 0,
                outpoint: OutPoint {
                    txid: txid_bytes.into(),
                    vout: 1,
                },
            },
        );
    }

    pub fn mempool(&self) -> BTreeMap<Txid, bitcoin::Transaction> {
        Decode!(
            &assert_reply(
                self.env()
                    .execute_ingress(self.bitcoin_id, "get_mempool", Encode!().unwrap())
                    .expect("failed to call get_mempool on the bitcoin mock")
            ),
            Vec<Vec<u8>>
        )
        .unwrap()
        .iter()
        .map(|tx_bytes| {
            let tx = bitcoin::Transaction::deserialize(tx_bytes)
                .expect("failed to parse a bitcoin transaction");

            (Txid::from(vec_to_txid(tx.txid().to_vec())), tx)
        })
        .collect()
    }

    pub fn minter_self_check(&self) {
        Decode!(
            &assert_reply(
                self.env()
                    .query(self.minter_id, "self_check", Encode!().unwrap())
                    .expect("failed to query self_check")
            ),
            Result<(), String>
        )
        .unwrap()
        .expect("minter self-check failed")
    }

    pub fn btc_to_eth20(&self, eth_address: &H160) -> Vec<Result<Erc20MintStatus, Erc20MintError>> {
        let payload = Encode!(eth_address).unwrap();
        let result = self
            .env()
            .execute_ingress(
                CanisterId::try_from(PrincipalId(self.context.canisters.btc_bridge())).unwrap(),
                "btc_to_erc20",
                payload,
            )
            .expect("btc_to_erc20 call failed");

        Decode!(
            &result.bytes(),
            Vec<Result<Erc20MintStatus, Erc20MintError>>
        )
        .expect("failed to decode btc_to_erc20 result")
    }

    pub fn advance_blocks(&self, blocks_count: usize) {
        for _ in 0..blocks_count {
            self.advance_tip_height(1);
            self.env().advance_time(Duration::from_secs(1));
        }
    }

    pub fn kyt_fee(&self) -> u64 {
        KYT_FEE
    }

    pub async fn async_drop(self) {
        let env = self.context.env;
        tokio::task::spawn_blocking(move || {
            drop(env);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn update_balance_should_return_correct_confirmations() {
    let ckbtc = CkBtcSetup::new().await;
    let upgrade_args = UpgradeArgs {
        retrieve_btc_min_amount: None,
        min_confirmations: Some(3),
        max_time_in_queue_nanos: None,
        mode: None,
        kyt_principal: None,
        kyt_fee: None,
    };
    let minter_arg = MinterArg::Upgrade(Some(upgrade_args));
    let env = ckbtc.env();
    let minter_id = ckbtc.minter_id;
    tokio::task::spawn_blocking(move || {
        env.upgrade_canister(minter_id, minter_wasm(), Encode!(&minter_arg).unwrap())
            .expect("Failed to upgrade the minter canister");
    })
    .await
    .unwrap();

    ckbtc.set_tip_height(12);

    let deposit_value = 100_000_000;
    let utxo = Utxo {
        height: 10,
        outpoint: OutPoint {
            txid: range_to_txid(1..=32).into(),
            vout: 1,
        },
        value: deposit_value,
    };

    let user = Principal::from(ckbtc.caller);

    ckbtc.deposit_utxo(user, utxo);

    let update_balance_args = UpdateBalanceArgs {
        owner: None,
        subaccount: None,
    };

    let env = ckbtc.env();
    let res = tokio::task::spawn_blocking(move || {
        env.execute_ingress_as(
            PrincipalId::new_user_test_id(1),
            ckbtc.minter_id,
            "update_balance",
            Encode!(&update_balance_args).unwrap(),
        )
        .expect("Failed to call update_balance")
    })
    .await
    .unwrap();

    let res = Decode!(&res.bytes(), Result<Vec<UtxoStatus>, UpdateBalanceError>).unwrap();
    assert_eq!(
        res,
        Err(UpdateBalanceError::NoNewUtxos {
            current_confirmations: None,
            required_confirmations: 3,
            pending_utxos: Some(vec![])
        })
    );

    ckbtc.async_drop().await;
}

#[tokio::test]
async fn btc_mint_flow() {
    let ckbtc = CkBtcSetup::new().await;

    ckbtc.set_tip_height(12);

    let deposit_value = 100_000_000;
    let utxo = Utxo {
        height: 12,
        outpoint: OutPoint {
            txid: range_to_txid(1..=32).into(),
            vout: 1,
        },
        value: deposit_value,
    };

    let wallet = (&ckbtc.context)
        .new_wallet(u128::MAX)
        .await
        .expect("Failed to create a wallet");
    let caller_eth_address = wallet.address().0.into();

    let deposit_account = Account {
        owner: ckbtc.context.canisters.btc_bridge(),
        subaccount: Some(eth_address_to_subaccount(&caller_eth_address).0),
    };
    let deposit_address = ckbtc.get_btc_address(deposit_account);
    ckbtc.push_utxo(deposit_address, utxo.clone());

    let result = ckbtc.btc_to_eth20(&caller_eth_address);
    assert_eq!(
        result[0],
        Ok(Erc20MintStatus::Scheduled {
            current_confirmations: 1,
            required_confirmations: MIN_CONFIRMATIONS,
            pending_utxos: Some(vec![PendingUtxo {
                outpoint: btc_bridge::ck_btc_interface::OutPoint {
                    txid: btc_bridge::ck_btc_interface::Txid::try_from(utxo.outpoint.txid.as_ref())
                        .unwrap(),
                    vout: utxo.outpoint.vout,
                },
                value: deposit_value,
                confirmations: 1,
            }])
        })
    );

    ckbtc.advance_blocks(6);

    let result = ckbtc.btc_to_eth20(&caller_eth_address);
    assert_eq!(
        result[0],
        Ok(Erc20MintStatus::Scheduled {
            current_confirmations: 7,
            required_confirmations: MIN_CONFIRMATIONS,
            pending_utxos: Some(vec![PendingUtxo {
                outpoint: btc_bridge::ck_btc_interface::OutPoint {
                    txid: btc_bridge::ck_btc_interface::Txid::try_from(utxo.outpoint.txid.as_ref())
                        .unwrap(),
                    vout: utxo.outpoint.vout,
                },
                value: deposit_value,
                confirmations: 7,
            }])
        })
    );

    ckbtc.advance_blocks(6);

    let result = ckbtc.btc_to_eth20(&caller_eth_address);
    assert_eq!(result[0], Err(Erc20MintError::NothingToMint));

    (&ckbtc.context).advance_time(Duration::from_secs(2)).await;

    if let Ok(Erc20MintStatus::Minted { tx_id, .. }) = &result[0] {
        let _receipt = (&ckbtc.context)
            .wait_transaction_receipt(tx_id)
            .await
            .unwrap();
    }

    let expected_balance = (deposit_value - ckbtc.kyt_fee() - CKBTC_LEDGER_FEE) as u128;
    let balance = (&ckbtc.context)
        .check_erc20_balance(&ckbtc.wrapped_token, &wallet)
        .await
        .unwrap();
    assert_eq!(balance, expected_balance);

    let canister_balance = ckbtc.balance_of(Account {
        owner: ckbtc.context.canisters.btc_bridge(),
        subaccount: None,
    });
    assert_eq!(canister_balance, expected_balance);

    ckbtc.async_drop().await;
}