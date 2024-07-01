mod evm_rpc_canister;

use std::collections::HashMap;
use std::time::Duration;

use candid::utils::ArgumentEncoder;
use candid::{Encode, Nat, Principal};
use did::constant::EIP1559_INITIAL_BASE_FEE;
use did::error::EvmError;
use did::init::EvmCanisterInitData;
use did::{NotificationInput, Transaction, TransactionReceipt, H160, H256, U256, U64};
use eth_signer::transaction::{SigningMethod, TransactionBuilder};
use eth_signer::{Signer, Wallet};
use ethers_core::abi::Token;
use ethers_core::k256::ecdsa::SigningKey;
use evm_canister_client::{CanisterClient, EvmCanisterClient};
use evm_rpc_canister::EvmRpcCanisterInitData;
use ic_exports::ic_kit::mock_principals::alice;
use ic_exports::icrc_types::icrc::generic_metadata_value::MetadataValue;
use ic_exports::icrc_types::icrc1::account::Account;
use ic_exports::icrc_types::icrc1_ledger::{
    ArchiveOptions, FeatureFlags, InitArgs, LedgerArgument,
};
use ic_exports::icrc_types::icrc2::approve::ApproveArgs;
use ic_log::LogSettings;
use icrc2_minter::SigningStrategy;
use icrc_client::IcrcCanisterClient;
use minter_contract_utils::build_data::{
    BFT_BRIDGE_SMART_CONTRACT_CODE, FEE_CHARGE_SMART_CONTRACT_CODE, UUPS_PROXY_SMART_CONTRACT_CODE,
};
use minter_contract_utils::evm_link::{address_to_icrc_subaccount, EvmLink};
use minter_contract_utils::fee_charge_api::{NATIVE_TOKEN_BALANCE, NATIVE_TOKEN_DEPOSIT};
use minter_contract_utils::{bft_bridge_api, fee_charge_api, wrapped_token_api};
use minter_did::error::Result as McResult;
use minter_did::id256::Id256;
use minter_did::init::InitData;
use minter_did::order::SignedMintOrder;
use minter_did::reason::{ApproveAfterMint, Icrc2Burn};
use tokio::time::Instant;

use super::utils::error::Result;
use crate::context::erc20_bridge_client::Erc20BridgeClient;
use crate::context::icrc2_bridge_client::Icrc2BridgeClient;
use crate::context::rune_bridge_client::RuneBridgeClient;
use crate::utils::btc::{BtcNetwork, InitArg, KytMode, LifecycleArg, MinterArg, Mode};
use crate::utils::error::TestError;
use crate::utils::wasm::*;
use crate::utils::{CHAIN_ID, EVM_PROCESSING_TRANSACTION_INTERVAL_FOR_TESTS};

pub const DEFAULT_GAS_PRICE: u128 = EIP1559_INITIAL_BASE_FEE * 2;

pub mod bridge_client;
pub mod erc20_bridge_client;
pub mod icrc2_bridge_client;
pub mod rune_bridge_client;

#[async_trait::async_trait]
pub trait TestContext {
    type Client: CanisterClient + Send + Sync;

    /// Returns principals for canisters in the context.
    fn canisters(&self) -> TestCanisters;

    /// Returns client for the canister.
    fn client(&self, canister: Principal, caller: &str) -> Self::Client;

    /// Returns principal by caller's name.
    fn principal_by_caller_name(&self, caller: &str) -> Principal;

    /// Principal to use for canisters initialization.
    fn admin(&self) -> Principal;

    /// Principal to use for canisters initialization.
    fn admin_name(&self) -> &str;

    /// Returns the base EVM LINK
    fn base_evm_link(&self) -> EvmLink {
        EvmLink::Ic(self.canisters().external_evm())
    }

    /// Returns client for the evm canister.
    fn evm_client(&self, caller: &str) -> EvmCanisterClient<Self::Client> {
        EvmCanisterClient::new(self.client(self.canisters().evm(), caller))
    }

    /// Returns client for the evm canister.
    fn icrc_minter_client(&self, caller: &str) -> Icrc2BridgeClient<Self::Client> {
        Icrc2BridgeClient::new(self.client(self.canisters().icrc2_minter(), caller))
    }

    fn erc_minter_client(&self, caller: &str) -> Erc20BridgeClient<Self::Client> {
        Erc20BridgeClient::new(self.client(self.canisters().ck_erc20_minter(), caller))
    }

    fn rune_bridge_client(&self, caller: &str) -> RuneBridgeClient<Self::Client> {
        RuneBridgeClient::new(self.client(self.canisters().rune_bridge(), caller))
    }

    /// Returns client for the ICRC token 1 canister.
    fn icrc_token_1_client(&self, caller: &str) -> IcrcCanisterClient<Self::Client> {
        IcrcCanisterClient::new(self.client(self.canisters().token_1(), caller))
    }

    /// Returns client for the ICRC token 2 canister.
    fn icrc_token_2_client(&self, caller: &str) -> IcrcCanisterClient<Self::Client> {
        IcrcCanisterClient::new(self.client(self.canisters().token_2(), caller))
    }

    /// Sends tx with notification to EVMc.
    async fn send_notification_tx(
        &self,
        user: &Wallet<SigningKey>,
        input: NotificationInput,
    ) -> Result<H256> {
        let address: H160 = user.address().into();
        let client = self.evm_client(self.admin_name());
        let account = client.account_basic(address.clone()).await?;

        let tx = self.signed_transaction(
            user,
            Some(address.clone()),
            account.nonce,
            0,
            input.encode().unwrap(),
        );

        Ok(client.send_raw_transaction(tx).await??)
    }

    /// Waits for transaction receipt.
    async fn wait_transaction_receipt(&self, hash: &H256) -> Result<Option<TransactionReceipt>> {
        let client = self.evm_client(self.admin_name());
        self.wait_transaction_receipt_on_evm(&client, hash).await
    }

    /// Waits for transaction receipt.
    async fn wait_transaction_receipt_on_evm(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        hash: &H256,
    ) -> Result<Option<TransactionReceipt>> {
        let tx_processing_interval = EVM_PROCESSING_TRANSACTION_INTERVAL_FOR_TESTS;
        let timeout = tx_processing_interval * 10;
        let start = Instant::now();
        let mut time_passed = Duration::ZERO;
        let mut receipt = None;
        while time_passed < timeout && receipt.is_none() {
            self.advance_time(tx_processing_interval).await;
            time_passed = Instant::now() - start;
            receipt = evm_client
                .eth_get_transaction_receipt(hash.clone())
                .await??;
        }
        Ok(receipt)
    }

    async fn advance_time(&self, time: Duration);

    /// Advances time by `duration` `times` times.
    async fn advance_by_times(&self, duration: Duration, times: u64) {
        for _ in 0..=times {
            self.advance_time(duration).await;
        }
    }

    /// Creates a new wallet with the EVM balance on it.
    async fn new_wallet(&self, balance: u128) -> Result<Wallet<'static, SigningKey>> {
        let wallet = {
            let mut rng = rand::thread_rng();
            Wallet::new(&mut rng)
        };
        let client = self.evm_client(self.admin_name());
        client
            .mint_native_tokens(wallet.address().into(), balance.into())
            .await??;

        self.advance_time(Duration::from_secs(2)).await;

        Ok(wallet)
    }

    /// Returns minter canister EVM address.
    async fn get_icrc_minter_canister_evm_address(&self, caller: &str) -> Result<H160> {
        let client = self.client(self.canisters().icrc2_minter(), caller);
        Ok(client
            .update::<_, McResult<H160>>("get_minter_canister_evm_address", ())
            .await??)
    }

    /// Creates contract in EVMc.
    async fn create_contract(
        &self,
        creator_wallet: &Wallet<'_, SigningKey>,
        input: Vec<u8>,
    ) -> Result<H160> {
        let evm_client = self.evm_client(self.admin_name());
        self.create_contract_on_evm(&evm_client, creator_wallet, input)
            .await
    }

    /// Creates contract on the given EVM.
    async fn create_contract_on_evm(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        creator_wallet: &Wallet<'_, SigningKey>,
        input: Vec<u8>,
    ) -> Result<H160> {
        let creator_address: H160 = creator_wallet.address().into();
        let nonce = evm_client
            .account_basic(creator_address.clone())
            .await
            .unwrap()
            .nonce;

        let create_contract_tx = self.signed_transaction(creator_wallet, None, nonce, 0, input);

        let hash = evm_client
            .send_raw_transaction(create_contract_tx)
            .await??;
        let receipt = self
            .wait_transaction_receipt_on_evm(evm_client, &hash)
            .await?
            .ok_or(TestError::Evm(EvmError::Internal(
                "transaction not processed".into(),
            )))?;

        if receipt.status != Some(U64::one()) {
            println!("tx status: {:?}", receipt.status);
            dbg!(&receipt);
            dbg!(&hex::encode(receipt.output.as_ref().unwrap_or(&vec![])));
            Err(TestError::Evm(EvmError::Internal(
                "contract creation failed".into(),
            )))
        } else {
            Ok(receipt.contract_address.expect(
                "contract creation transaction succeeded, but it doesn't contain the contract address",
            ))
        }
    }

    /// Crates BFTBridge contract in EVMc and registered it in minter canister
    async fn initialize_bft_bridge(&self, caller: &str, fee_charge_address: H160) -> Result<H160> {
        let minter_canister_address = self.get_icrc_minter_canister_evm_address(caller).await?;

        let client = self.evm_client(self.admin_name());
        client
            .mint_native_tokens(minter_canister_address.clone(), u64::MAX.into())
            .await??;
        self.advance_time(Duration::from_secs(2)).await;

        let bridge_address = self
            .initialize_bft_bridge_with_minter(
                &self.new_wallet(u64::MAX.into()).await?,
                minter_canister_address,
                Some(fee_charge_address),
                true,
            )
            .await?;

        let raw_client = self.client(self.canisters().icrc2_minter(), self.admin_name());
        raw_client
            .update("set_bft_bridge_contract", (bridge_address.clone(),))
            .await?;

        Ok(bridge_address)
    }

    /// Creates BFTBridge contract in EVMC and registered it in minter canister
    async fn initialize_bft_bridge_with_minter(
        &self,
        wallet: &Wallet<'_, SigningKey>,
        minter_canister_address: H160,
        fee_charge_address: Option<H160>,
        is_wrapped: bool,
    ) -> Result<H160> {
        let contract = BFT_BRIDGE_SMART_CONTRACT_CODE.clone();
        let input = bft_bridge_api::CONSTRUCTOR
            .encode_input(contract, &[])
            .unwrap();

        let bridge_address = self.create_contract(wallet, input.clone()).await.unwrap();

        let initialize_data = bft_bridge_api::proxy::INITIALISER
            .encode_input(&[
                Token::Address(minter_canister_address.0),
                Token::Address(fee_charge_address.unwrap_or_default().0),
                Token::Bool(is_wrapped),
            ])
            .expect("encode input");

        let proxy_input = bft_bridge_api::proxy::CONSTRUCTOR
            .encode_input(
                UUPS_PROXY_SMART_CONTRACT_CODE.clone(),
                &[
                    Token::Address(bridge_address.0),
                    Token::Bytes(initialize_data),
                ],
            )
            .unwrap();

        let proxy_address = self.create_contract(wallet, proxy_input).await.unwrap();

        Ok(proxy_address)
    }

    async fn initialize_fee_charge_contract(
        &self,
        wallet: &Wallet<'_, SigningKey>,
        minter_canister_addresses: &[H160],
    ) -> Result<H160> {
        let evm = self.evm_client(self.admin_name());
        self.initialize_fee_charge_contract_on_evm(&evm, wallet, minter_canister_addresses)
            .await
    }

    async fn initialize_fee_charge_contract_on_evm(
        &self,
        evm: &EvmCanisterClient<Self::Client>,
        wallet: &Wallet<'_, SigningKey>,
        minter_canister_addresses: &[H160],
    ) -> Result<H160> {
        let contract = FEE_CHARGE_SMART_CONTRACT_CODE.clone();
        let minter_canister_addresses = minter_canister_addresses
            .iter()
            .map(|addr| Token::Address(addr.0))
            .collect();
        let input = fee_charge_api::CONSTRUCTOR
            .encode_input(contract, &[Token::Array(minter_canister_addresses)])
            .unwrap();

        let fee_charge_address = self
            .create_contract_on_evm(evm, wallet, input.clone())
            .await
            .unwrap();

        Ok(fee_charge_address)
    }

    #[allow(clippy::too_many_arguments)]
    async fn burn_erc_20_tokens_raw(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        wallet: &Wallet<'_, SigningKey>,
        from_token: &H160,
        to_token_id: &[u8],
        recipient: Vec<u8>,
        bridge: &H160,
        amount: u128,
    ) -> Result<(u32, H256)> {
        let amount = amount.into();
        let input = wrapped_token_api::ERC_20_APPROVE
            .encode_input(&[Token::Address(bridge.0), Token::Uint(amount)])
            .unwrap();

        let results = self
            .call_contract_on_evm(evm_client, wallet, from_token, input, 0)
            .await?;
        let output = results.1.output.unwrap();
        assert_eq!(
            wrapped_token_api::ERC_20_APPROVE
                .decode_output(&output)
                .unwrap()[0],
            Token::Bool(true)
        );

        println!("Burning src tokens using BftBridge");
        let input = bft_bridge_api::BURN
            .encode_input(&[
                Token::Uint(amount),
                Token::Address(from_token.0),
                Token::FixedBytes(to_token_id.to_vec()),
                Token::Bytes(recipient),
            ])
            .unwrap();

        let (tx_hash, receipt) = self
            .call_contract_on_evm(evm_client, wallet, bridge, input, 0)
            .await?;
        let decoded_output = bft_bridge_api::BURN
            .decode_output(receipt.output.as_ref().unwrap())
            .unwrap();
        if receipt.status != Some(U64::one()) {
            return Err(TestError::Generic(format!(
                "Burn transaction failed: {decoded_output:?} -- {receipt:?}, -- {}",
                String::from_utf8_lossy(receipt.output.as_ref().unwrap())
            )));
        }

        let operation_id = decoded_output[0].clone().into_uint().unwrap().as_u32();
        Ok((operation_id, tx_hash))
    }

    #[allow(clippy::too_many_arguments)]
    async fn burn_erc_20_tokens(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        wallet: &Wallet<'_, SigningKey>,
        from_token: &H160,
        to_token_id: &[u8],
        recipient: Id256,
        bridge: &H160,
        amount: u128,
    ) -> Result<(u32, H256)> {
        self.burn_erc_20_tokens_raw(
            evm_client,
            wallet,
            from_token,
            to_token_id,
            recipient.0.to_vec(),
            bridge,
            amount,
        )
        .await
    }

    /// Current native token balance on user's deposit inside the BftBridge.
    async fn native_token_deposit_balance(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        fee_charge: H160,
        user: H160,
    ) -> U256 {
        let input = NATIVE_TOKEN_BALANCE
            .encode_input(&[Token::Address(user.0)])
            .unwrap();
        let response = evm_client
            .eth_call(
                Some(user),
                Some(fee_charge),
                None,
                3_000_000,
                None,
                Some(input.into()),
            )
            .await
            .unwrap()
            .unwrap();

        NATIVE_TOKEN_BALANCE
            .decode_output(&hex::decode(response.trim_start_matches("0x")).unwrap())
            .unwrap()
            .first()
            .cloned()
            .unwrap()
            .into_uint()
            .unwrap()
            .into()
    }

    /// Deposit native tokens to BftBridge to pay mint fee.
    async fn native_token_deposit(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        fee_charge: H160,
        user_wallet: &Wallet<'static, SigningKey>,
        sender_ids: &[Id256],
        amount: u128,
    ) -> Result<U256> {
        let sender_ids = sender_ids
            .iter()
            .map(|id| Token::FixedBytes(id.0.to_vec()))
            .collect();
        let input = NATIVE_TOKEN_DEPOSIT
            .encode_input(&[Token::Array(sender_ids)])
            .unwrap();

        let receipt = self
            .call_contract_on_evm(evm_client, user_wallet, &fee_charge, input, amount)
            .await?
            .1;

        let new_balance = NATIVE_TOKEN_DEPOSIT
            .decode_output(receipt.output.as_ref().unwrap())
            .unwrap()
            .first()
            .cloned()
            .unwrap()
            .into_uint()
            .unwrap();

        Ok(new_balance.into())
    }

    /// Returns a signed transaction from the given `wallet`.
    fn signed_transaction(
        &self,
        wallet: &Wallet<SigningKey>,
        to: Option<H160>,
        nonce: U256,
        value: u128,
        input: Vec<u8>,
    ) -> Transaction {
        let address = wallet.address();
        TransactionBuilder {
            from: &address.into(),
            to,
            nonce,
            value: value.into(),
            gas: 5_000_000u64.into(),
            gas_price: Some(DEFAULT_GAS_PRICE.into()),
            input,
            signature: SigningMethod::SigningKey(wallet.signer()),
            chain_id: CHAIN_ID,
        }
        .calculate_hash_and_build()
        .unwrap()
    }

    /// Calls contract in EVMc.
    async fn call_contract(
        &self,
        wallet: &Wallet<'_, SigningKey>,
        contract: &H160,
        input: Vec<u8>,
        amount: u128,
    ) -> Result<(H256, TransactionReceipt)> {
        let evm_client = self.evm_client(self.admin_name());
        self.call_contract_on_evm(&evm_client, wallet, contract, input, amount)
            .await
    }

    /// Calls contract in the evm_client.
    async fn call_contract_on_evm(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        wallet: &Wallet<'_, SigningKey>,
        contract: &H160,
        input: Vec<u8>,
        amount: u128,
    ) -> Result<(H256, TransactionReceipt)> {
        let from: H160 = wallet.address().into();
        let nonce = evm_client.account_basic(from.clone()).await?.nonce;

        let call_tx = self.signed_transaction(wallet, Some(contract.clone()), nonce, amount, input);

        let hash = evm_client.send_raw_transaction(call_tx).await??;
        let receipt = self
            .wait_transaction_receipt_on_evm(evm_client, &hash)
            .await?
            .ok_or(TestError::Evm(EvmError::Internal(
                "transaction not processed".into(),
            )))?;

        if receipt.status != Some(U64::one()) {
            println!("tx status: {:?}", receipt.status);
            dbg!(&receipt);
            dbg!(&hex::encode(receipt.output.as_ref().unwrap_or(&vec![])));
        }

        Ok((hash, receipt))
    }

    /// Creates wrapped token in EVMc by calling `BFTBridge:::deploy_wrapped_token()`.
    async fn create_wrapped_token(
        &self,
        wallet: &Wallet<'_, SigningKey>,
        bft_bridge: &H160,
        base_token_id: Id256,
    ) -> Result<H160> {
        let input = bft_bridge_api::DEPLOY_WRAPPED_TOKEN
            .encode_input(&[
                Token::String("wrapper".into()),
                Token::String("WPT".into()),
                Token::FixedBytes(base_token_id.0.to_vec()),
            ])
            .unwrap();

        let results = self.call_contract(wallet, bft_bridge, input, 0).await?;
        let output = results.1.output.unwrap();

        let token_address = bft_bridge_api::DEPLOY_WRAPPED_TOKEN
            .decode_output(&output)
            .unwrap()[0]
            .clone()
            .into_address()
            .unwrap();
        println!(
            "Deployed Wrapped token on block {} with address {token_address}",
            results.1.block_number
        );

        Ok(token_address.into())
    }

    /// Burns ICRC-2 token 1 and creates according mint order.
    #[allow(clippy::too_many_arguments)]
    async fn burn_icrc2(
        &self,
        caller: &str,
        wallet: &Wallet<'_, SigningKey>,
        bridge: &H160,
        erc20_token_address: &H160,
        amount: u128,
        fee_payer: Option<H160>,
        approve_after_mint: Option<ApproveAfterMint>,
    ) -> Result<()> {
        let recipient_address = H160::from(wallet.address());
        self.approve_icrc2_burn(
            caller,
            &recipient_address,
            amount + ICRC1_TRANSFER_FEE as u128,
        )
        .await?;

        let reason = Icrc2Burn {
            sender: self.principal_by_caller_name(caller),
            amount: amount.into(),
            from_subaccount: None,
            icrc2_token_principal: self.canisters().token_1(),
            erc20_token_address: erc20_token_address.clone(),
            recipient_address,
            fee_payer,
            approve_after_mint,
        };

        let encoded_reason = Encode!(&reason).unwrap();

        let input = bft_bridge_api::NOTIFY_MINTER
            .encode_input(&[
                Token::Uint(Default::default()),
                Token::Bytes(encoded_reason),
            ])
            .unwrap();
        let _receipt = self
            .call_contract(wallet, bridge, input, 0)
            .await
            .map(|(_, receipt)| receipt)?;

        Ok(())
    }

    /// Approves burning of ICRC-2 token.
    async fn approve_icrc2_burn(&self, caller: &str, recipient: &H160, amount: u128) -> Result<()> {
        let client = self.icrc_token_1_client(caller);

        let subaccount = Some(address_to_icrc_subaccount(&recipient.0));
        let minter_canister = Account {
            owner: self.canisters().icrc2_minter(),
            subaccount,
        };

        let approve_args = ApproveArgs {
            from_subaccount: None,
            spender: minter_canister,
            amount: amount.into(),
            expected_allowance: None,
            expires_at: None,
            fee: None,
            memo: None,
            created_at_time: None,
        };

        client.icrc2_approve(approve_args).await?.unwrap();
        Ok(())
    }

    /// Mints ERC-20 token with the order.
    async fn mint_erc_20_with_order(
        &self,
        wallet: &Wallet<'_, SigningKey>,
        bridge: &H160,
        order: SignedMintOrder,
    ) -> Result<TransactionReceipt> {
        let input = bft_bridge_api::MINT
            .encode_input(&[Token::Bytes(order.0.to_vec())])
            .unwrap();
        self.call_contract(wallet, bridge, input, 0)
            .await
            .map(|(_, receipt)| receipt)
    }

    /// Returns ERC-20 balance.
    async fn check_erc20_balance(
        &self,
        token: &H160,
        wallet: &Wallet<'_, SigningKey>,
        address: Option<&H160>,
    ) -> Result<u128> {
        let evm_client = self.evm_client(self.admin_name());
        self.check_erc20_balance_on_evm(&evm_client, token, wallet, address)
            .await
    }

    /// Returns ERC-20 balance on the given evm.
    async fn check_erc20_balance_on_evm(
        &self,
        evm_client: &EvmCanisterClient<Self::Client>,
        token: &H160,
        wallet: &Wallet<'_, SigningKey>,
        address: Option<&H160>,
    ) -> Result<u128> {
        let input = wrapped_token_api::ERC_20_BALANCE
            .encode_input(&[Token::Address(
                address
                    .cloned()
                    .unwrap_or_else(|| wallet.address().into())
                    .into(),
            )])
            .unwrap();
        let results = self
            .call_contract_on_evm(evm_client, wallet, token, input, 0)
            .await?;
        let output = results.1.output.unwrap();
        println!("output: {:?}", hex::encode(&output));

        Ok(wrapped_token_api::ERC_20_BALANCE
            .decode_output(&output)
            .unwrap()[0]
            .clone()
            .into_uint()
            .unwrap()
            .as_u128())
    }

    /// Creates an empty canister with cycles on it's balance.
    async fn create_canister(&self) -> Result<Principal>;

    async fn create_canister_with_id(&self, id: Principal) -> Result<Principal>;

    /// Installs the `wasm` code to the `canister` with the given init `args`.
    async fn install_canister(
        &self,
        canister: Principal,
        wasm: Vec<u8>,
        args: impl ArgumentEncoder + Send,
    ) -> Result<()>;

    /// Reinstalls the canister.
    async fn reinstall_canister(
        &self,
        canister: Principal,
        wasm: Vec<u8>,
        args: impl ArgumentEncoder + Send,
    ) -> Result<()>;

    /// Upgrades the canister.
    async fn upgrade_canister(
        &self,
        canister: Principal,
        wasm: Vec<u8>,
        args: impl ArgumentEncoder + Send,
    ) -> Result<()>;

    /// Installs code to test context's canister with the given type.
    /// If the canister depends on not-created canister, Principal::anonymous() is used.
    async fn install_default_canister(&self, canister_type: CanisterType) {
        let wasm = canister_type.default_canister_wasm().await;
        match canister_type {
            CanisterType::Evm => {
                println!("Installing default EVM canister...");
                let signature_canister = self.canisters().get_or_anonymous(CanisterType::Signature);
                let init_data = evm_canister_init_data(
                    signature_canister,
                    self.admin(),
                    Some(EVM_PROCESSING_TRANSACTION_INTERVAL_FOR_TESTS),
                );
                self.install_canister(self.canisters().evm(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::ExternalEvm => {
                println!("Installing external EVM canister...");
                let signature_canister = self.canisters().get_or_anonymous(CanisterType::Signature);
                let init_data = evm_canister_init_data(
                    signature_canister,
                    self.admin(),
                    Some(EVM_PROCESSING_TRANSACTION_INTERVAL_FOR_TESTS),
                );
                self.install_canister(self.canisters().external_evm(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::EvmRpcCanister => {
                println!(
                    "Installing default EVM RPC canister {}...",
                    self.canisters().evm_rpc()
                );
                let init_data = EvmRpcCanisterInitData { nodesInSubnet: 1 };
                self.install_canister(self.canisters().evm_rpc(), wasm, (init_data,))
                    .await
                    .unwrap();

                let client = self.client(self.canisters().evm_rpc(), self.admin_name());

                let res = client
                    .update::<_, bool>(
                        "authorize",
                        (self.admin(), evm_rpc_canister::Auth::RegisterProvider),
                    )
                    .await
                    .expect("authorize failed");
                assert!(res, "authorize failed");
                let hostname = format!(
                    "https://127.0.0.1:8002/?canisterId={}",
                    self.canisters().external_evm()
                );
                println!("EVM-RPC provider hostname: {hostname}");
                // configure the EVM RPC canister provider
                let args = evm_rpc_canister::RegisterProviderArgs {
                    chainId: CHAIN_ID,
                    hostname,
                    credentialPath: "".to_string(),
                    cyclesPerCall: 1,
                    cyclesPerMessageByte: 1,
                    credentialsHeaders: None,
                };

                client
                    .update::<_, u64>("registerProvider", (args,))
                    .await
                    .expect("registerProvider failed");
            }
            CanisterType::Signature => {
                println!("Installing default Signature canister...");
                let possible_canisters = [CanisterType::Evm, CanisterType::ExternalEvm];
                let init_data = possible_canisters
                    .into_iter()
                    .filter_map(|canister_type| self.canisters().get(canister_type))
                    .collect::<Vec<_>>();

                self.install_canister(
                    self.canisters().signature_verification(),
                    wasm,
                    (init_data,),
                )
                .await
                .unwrap();
            }
            CanisterType::Token1 => {
                println!("Installing default Token1 canister...");
                let init_balances = self.icrc_token_initial_balances();
                let init_data =
                    icrc_canister_default_init_args(self.admin(), "Tokenium", init_balances);
                self.install_canister(
                    self.canisters().token_1(),
                    wasm,
                    (LedgerArgument::Init(init_data),),
                )
                .await
                .unwrap();
            }
            CanisterType::Token2 => {
                println!("Installing default Token2 canister...");
                let init_balances = self.icrc_token_initial_balances();
                let init_data =
                    icrc_canister_default_init_args(self.admin(), "Tokenium 2", init_balances);
                self.install_canister(
                    self.canisters().token_2(),
                    wasm,
                    (LedgerArgument::Init(init_data),),
                )
                .await
                .unwrap();
            }
            CanisterType::Icrc2Minter => {
                println!("Installing default Minter canister...");
                let evm_canister = self.canisters().get_or_anonymous(CanisterType::Evm);
                let init_data = minter_canister_init_data(self.admin(), evm_canister);
                self.install_canister(self.canisters().icrc2_minter(), wasm, (init_data,))
                    .await
                    .unwrap();

                // Wait for initialization of the Minter canister parameters.
                self.advance_time(Duration::from_secs(2)).await;
            }
            CanisterType::Icrc1Ledger => {
                println!("Installing default ICRC1 ledger canister...");
                let init_data = icrc1_ledger_init_data(self.canisters().ck_btc_minter());
                self.install_canister(self.canisters().icrc1_ledger(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::CkBtcMinter => {
                println!("Installing default ckBTC minter canister...");
                let init_data = ck_btc_minter_init_data(
                    self.canisters().icrc1_ledger(),
                    self.canisters().kyt(),
                );
                self.install_canister(self.canisters().ck_btc_minter(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::Btc => {
                println!("Installing default mock ckBTC canister...");
                todo!()
            }
            CanisterType::Kyt => {
                println!("Installing default KYT canister...");
                let init_data = kyc_init_data(self.canisters().ck_btc_minter());
                self.install_canister(self.canisters().kyt(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::CkErc20Minter => {
                let evm_canister = self.canisters().evm();
                let init_data = erc20_minter::state::Settings {
                    base_evm_link: self.base_evm_link(),
                    wrapped_evm_link: EvmLink::Ic(evm_canister),
                    signing_strategy: SigningStrategy::Local {
                        private_key: rand::random(),
                    },
                    log_settings: Some(LogSettings {
                        enable_console: true,
                        in_memory_records: None,
                        log_filter: Some("trace".to_string()),
                    }),
                };
                self.install_canister(self.canisters().ck_erc20_minter(), wasm, (init_data,))
                    .await
                    .unwrap();
            }
            CanisterType::BtcBridge => {
                todo!()
            }
            CanisterType::RuneBridge => {}
        }
    }

    /// Reinstall the EVM canister with default settings.
    async fn reinstall_evm_canister(
        &self,
        transaction_processing_interval: Option<Duration>,
    ) -> Result<()> {
        let init_data = evm_canister_init_data(
            self.canisters().signature_verification(),
            self.admin(),
            transaction_processing_interval,
        );
        let wasm = get_evm_testnet_canister_bytecode().await;
        self.reinstall_canister(self.canisters().evm(), wasm, (init_data,))
            .await?;

        Ok(())
    }

    /// Upgrades the EVM canister with default settings.
    async fn upgrade_evm_canister(&self) -> Result<()> {
        let wasm = get_evm_testnet_canister_bytecode().await;
        self.upgrade_canister(self.canisters().evm(), wasm, ())
            .await?;
        Ok(())
    }

    /// Upgrades the minter canister with default settings.
    async fn upgrade_minter_canister(&self) -> Result<()> {
        let wasm = get_icrc2_minter_canister_bytecode().await;
        self.upgrade_canister(self.canisters().icrc2_minter(), wasm, ())
            .await?;
        Ok(())
    }

    /// Reinstalls the icrc1 token canister with default settings.
    async fn reinstall_icrc1_canister(
        &self,
        token_canister: Principal,
        token_name: &str,
        initial_balances: Vec<(Account, Nat)>,
    ) -> Result<()> {
        let init_args = icrc_canister_default_init_args(self.admin(), token_name, initial_balances);
        let args = LedgerArgument::Init(init_args);
        let wasm = get_icrc1_token_canister_bytecode().await;
        self.reinstall_canister(token_canister, wasm, (args,))
            .await?;

        Ok(())
    }

    async fn reinstall_minter_canister(&self) -> Result<()> {
        eprintln!("reinstalling Minter canister");
        let init_data = minter_canister_init_data(self.admin(), self.canisters().evm());

        let wasm = get_icrc2_minter_canister_bytecode().await;
        self.reinstall_canister(self.canisters().icrc2_minter(), wasm, (init_data,))
            .await?;

        Ok(())
    }

    fn icrc_token_initial_balances(&self) -> Vec<(Account, Nat)>;
}

pub const ICRC1_TRANSFER_FEE: u64 = 10_000;
pub const ICRC1_INITIAL_BALANCE: u64 = 10u64.pow(18);

pub fn icrc_canister_default_init_args(
    caller: Principal,
    token_name: &str,
    initial_balances: Vec<(Account, Nat)>,
) -> InitArgs {
    InitArgs {
        minting_account: Account::from(caller),
        fee_collector_account: None,
        initial_balances,
        transfer_fee: Nat::from(ICRC1_TRANSFER_FEE),
        token_name: token_name.to_string(),
        token_symbol: "TKN".to_string(),
        metadata: vec![(
            "icrc1:name".to_string(),
            MetadataValue::Text(token_name.to_string()),
        )],
        archive_options: ArchiveOptions {
            trigger_threshold: 10,
            num_blocks_to_archive: 5,
            node_max_memory_size_bytes: None,
            max_message_size_bytes: None,
            controller_id: caller,
            cycles_for_archive_creation: None,
            max_transactions_per_response: None,
        },
        max_memo_length: None,
        feature_flags: Some(FeatureFlags { icrc2: true }),
        decimals: None,
        maximum_number_of_accounts: None,
        accounts_overflow_trim_quantity: None,
    }
}

pub fn minter_canister_init_data(owner: Principal, evm_principal: Principal) -> InitData {
    let mut rng = rand::thread_rng();
    let wallet = Wallet::new(&mut rng);
    InitData {
        owner,
        evm_principal,
        signing_strategy: SigningStrategy::Local {
            private_key: wallet.signer().to_bytes().into(),
        },
        log_settings: Some(LogSettings {
            enable_console: true,
            in_memory_records: None,
            log_filter: Some("trace".to_string()),
        }),
    }
}

pub fn evm_canister_init_data(
    signature_verification_principal: Principal,
    owner: Principal,
    transaction_processing_interval: Option<Duration>,
) -> EvmCanisterInitData {
    EvmCanisterInitData {
        signature_verification_principal,
        min_gas_price: 10_u64.into(),
        chain_id: CHAIN_ID,
        log_settings: Some(LogSettings {
            enable_console: true,
            in_memory_records: None,
            log_filter: Some("debug".to_string()),
        }),
        transaction_processing_interval,
        owner,
        ..Default::default()
    }
}

fn icrc1_ledger_init_data(minter_principal: Principal) -> LedgerArgument {
    let minting_account = Account {
        owner: minter_principal,
        subaccount: None,
    };
    let archive_options = ArchiveOptions {
        trigger_threshold: 10_000_000,
        num_blocks_to_archive: 1_000_000,
        node_max_memory_size_bytes: None,
        max_message_size_bytes: None,
        controller_id: Principal::anonymous(),
        cycles_for_archive_creation: None,
        max_transactions_per_response: None,
    };

    LedgerArgument::Init(InitArgs {
        minting_account,
        fee_collector_account: None,
        initial_balances: vec![],
        transfer_fee: Nat::from(10u32),
        decimals: None,
        token_name: "ckBtc".into(),
        token_symbol: "ckBtc".into(),
        metadata: vec![],
        archive_options,
        max_memo_length: Some(80),
        feature_flags: Some(FeatureFlags { icrc2: true }),
        maximum_number_of_accounts: None,
        accounts_overflow_trim_quantity: None,
    })
}

fn ck_btc_minter_init_data(ledger: Principal, kyt: Principal) -> MinterArg {
    MinterArg::Init(crate::utils::btc::InitArgs {
        btc_network: BtcNetwork::Mainnet,
        ecdsa_key_name: "master_ecdsa_public_key_fscpm-uiaaa-aaaaa-aaaap-yai".to_string(),
        retrieve_btc_min_amount: 100_000,
        ledger_id: ledger,
        max_time_in_queue_nanos: 100,
        min_confirmations: Some(12),
        mode: Mode::GeneralAvailability,
        kyt_fee: Some(2000),
        kyt_principal: Some(kyt),
    })
}

fn kyc_init_data(ck_btc_minter: Principal) -> LifecycleArg {
    LifecycleArg::InitArg(InitArg {
        minter_id: ck_btc_minter,
        maintainers: vec![alice()],
        mode: KytMode::AcceptAll,
    })
}

#[derive(Debug, Clone, Default)]
pub struct TestCanisters(HashMap<CanisterType, Principal>);

impl TestCanisters {
    pub fn token_1(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Token1)
            .expect("token_1 canister should be initialized (see `TestContext::new()`)")
    }

    pub fn token_2(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Token2)
            .expect("token_2 canister should be initialized (see `TestContext::new()`)")
    }

    pub fn evm(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Evm)
            .expect("evm canister should be initialized (see `TestContext::new()`)")
    }

    pub fn external_evm(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::ExternalEvm)
            .expect("external evm canister should be initialized (see `TestContext::new()`)")
    }

    pub fn evm_rpc(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::EvmRpcCanister)
            .expect("evm rpc canister should be initialized (see `TestContext::new()`)")
    }

    pub fn signature_verification(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Signature)
            .expect("signature canister should be initialized (see `TestContext::new()`)")
    }

    pub fn icrc2_minter(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Icrc2Minter)
            .expect("icrc2 minter canister should be initialized (see `TestContext::new()`)")
    }

    pub fn ck_erc20_minter(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::CkErc20Minter)
            .expect("ck erc20 minter canister should be initialized (see `TestContext::new()`)")
    }

    pub fn ck_btc_minter(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::CkBtcMinter)
            .expect("ckBTC minter canister should be initialized (see `TestContext::new()`)")
    }

    pub fn btc_mock(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Btc)
            .expect("bitcoin mock canister should be initialized (see `TestContext::new()`)")
    }

    pub fn icrc1_ledger(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Icrc1Ledger)
            .expect("icrc1 ledger canister should be initialized (see `TestContext::new()`)")
    }

    pub fn kyt(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::Kyt)
            .expect("kyt canister should be initialized (see `TestContext::new()`)")
    }

    pub fn btc_bridge(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::BtcBridge)
            .expect("bridge canister should be initialized (see `TestContext::new()`)")
    }

    pub fn rune_bridge(&self) -> Principal {
        *self
            .0
            .get(&CanisterType::RuneBridge)
            .expect("rune bridge canister should be initialized (see `TestContext::new()`)")
    }

    pub fn set(&mut self, canister_type: CanisterType, principal: Principal) {
        self.0.insert(canister_type, principal);
    }

    pub fn get(&self, canister_type: CanisterType) -> Option<Principal> {
        self.0.get(&canister_type).copied()
    }

    pub fn get_or_anonymous(&self, canister_type: CanisterType) -> Principal {
        self.0
            .get(&canister_type)
            .copied()
            .unwrap_or_else(Principal::anonymous)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanisterType {
    Evm,
    EvmRpcCanister,
    ExternalEvm,
    Signature,
    Token1,
    Token2,
    Icrc2Minter,
    CkErc20Minter,
    Btc,
    CkBtcMinter,
    Kyt,
    Icrc1Ledger,
    BtcBridge,
    RuneBridge,
}

impl CanisterType {
    /// EVM and SignatureVerification.
    pub const EVM_TEST_SET: [CanisterType; 2] = [CanisterType::Evm, CanisterType::Signature];

    /// EVM, SignatureVerification, Minter and Token1.
    pub const ICRC2_MINTER_TEST_SET: [CanisterType; 4] = [
        CanisterType::Evm,
        CanisterType::Signature,
        CanisterType::Token1,
        CanisterType::Icrc2Minter,
    ];

    /// EVM, SignatureVerification, Minter, Spender and Token1.
    pub const EVM_MINTER_TEST_SET: [CanisterType; 4] = [
        CanisterType::Evm,
        CanisterType::ExternalEvm,
        CanisterType::Signature,
        CanisterType::CkErc20Minter,
    ];

    /// EVM, SignatureVerification, Minter, Spender and Token1.
    pub const EVM_MINTER_WITH_EVMRPC_TEST_SET: [CanisterType; 5] = [
        CanisterType::Evm,
        CanisterType::ExternalEvm,
        CanisterType::EvmRpcCanister,
        CanisterType::Signature,
        CanisterType::CkErc20Minter,
    ];

    pub const BTC_CANISTER_SET: [CanisterType; 4] = [
        CanisterType::Btc,
        CanisterType::CkBtcMinter,
        CanisterType::Kyt,
        CanisterType::Icrc1Ledger,
    ];

    pub const RUNE_CANISTER_SET: [CanisterType; 3] = [
        CanisterType::Evm,
        CanisterType::Signature,
        CanisterType::RuneBridge,
    ];

    pub async fn default_canister_wasm(&self) -> Vec<u8> {
        match self {
            CanisterType::Evm => get_evm_testnet_canister_bytecode().await,
            CanisterType::EvmRpcCanister => get_evm_rpc_canister_bytecode().await,
            CanisterType::ExternalEvm => get_evm_testnet_canister_bytecode().await,
            CanisterType::Signature => get_signature_verification_canister_bytecode().await,
            CanisterType::Token1 => get_icrc1_token_canister_bytecode().await,
            CanisterType::Token2 => get_icrc1_token_canister_bytecode().await,
            CanisterType::Icrc2Minter => get_icrc2_minter_canister_bytecode().await,
            CanisterType::CkErc20Minter => get_ck_erc20_minter_canister_bytecode().await,
            CanisterType::Btc => get_btc_canister_bytecode().await,
            CanisterType::CkBtcMinter => get_ck_btc_minter_canister_bytecode().await,
            CanisterType::Kyt => get_kyt_canister_bytecode().await,
            CanisterType::Icrc1Ledger => get_icrc1_token_canister_bytecode().await,
            CanisterType::BtcBridge => get_btc_bridge_canister_bytecode().await,
            CanisterType::RuneBridge => get_rune_bridge_canister_bytecode().await,
        }
    }
}
