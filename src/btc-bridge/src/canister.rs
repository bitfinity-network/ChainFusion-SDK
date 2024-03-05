use std::cell::RefCell;
use std::rc::Rc;

use crate::ck_btc_interface::{UpdateBalanceArgs, UpdateBalanceError, UtxoStatus};
use crate::interface::Erc20MintStatus::Scheduled;
use crate::interface::{Erc20MintError, Erc20MintStatus};
use candid::{CandidType, Principal};
use did::{H160, H256};
use eth_signer::sign_strategy::TransactionSigner;
use ic_canister::{generate_idl, virtual_canister_call, Canister, Idl, PreUpdate};
use ic_exports::ic_cdk::api::management_canister::bitcoin::Utxo;
use ic_exports::ledger::Subaccount;
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::CellStructure;
use minter_did::id256::Id256;
use minter_did::order::{MintOrder, SignedMintOrder};
use serde::Deserialize;

use crate::state::{BtcBridgeConfig, State};

#[derive(Canister, Clone, Debug)]
pub struct BtcBridge {
    #[id]
    id: Principal,
}

impl PreUpdate for BtcBridge {}

#[derive(Debug, CandidType, Deserialize)]
pub struct InitArgs {
    ck_btc_minter: Principal,
    ck_btc_ledger: Principal,
}

impl BtcBridge {
    // #[init]
    pub fn init(&mut self, config: BtcBridgeConfig) {
        get_state().borrow_mut().configure(config);
    }

    // #[update]
    pub async fn btc_to_erc20(&self, eth_address: H160) -> Result<Erc20MintStatus, Erc20MintError> {
        match self.request_update_balance(&eth_address).await {
            Ok(UtxoStatus::Minted {
                minted_amount,
                utxo,
                ..
            }) => self.mint_erc20(eth_address, minted_amount, utxo).await,
            Err(UpdateBalanceError::NoNewUtxos {
                current_confirmations: Some(curr_confirmations),
                required_confirmations,
                pending_utxos,
            }) => {
                self.schedule_mint(eth_address);
                Ok(Scheduled {
                    current_confirmations: curr_confirmations,
                    required_confirmations,
                    pending_utxos,
                })
            }
            Ok(UtxoStatus::ValueTooSmall(utxo)) => Err(Erc20MintError::ValueTooSmall(utxo)),
            Ok(UtxoStatus::Tainted(utxo)) => Err(Erc20MintError::Tainted(utxo)),
            Ok(UtxoStatus::Checked(_)) => Err(Erc20MintError::CkBtcError(
                UpdateBalanceError::TemporarilyUnavailable(
                    "KYT check passed, but mint failed. Try again later.".to_string(),
                ),
            )),
            Err(err) => Err(Erc20MintError::CkBtcError(err)),
        }
    }

    async fn request_update_balance(
        &self,
        eth_address: &H160,
    ) -> Result<UtxoStatus, UpdateBalanceError> {
        let self_id = self.id;
        let ck_btc_minter = get_state().borrow().ck_btc_minter();
        let subaccount = eth_address_to_subaccount(eth_address);

        let args = UpdateBalanceArgs {
            owner: Some(self_id),
            subaccount: Some(subaccount),
        };
        virtual_canister_call!(ck_btc_minter, "update_balance", (args,), Result<UtxoStatus, UpdateBalanceError>)
            .await
            .unwrap_or_else(|err| Err(UpdateBalanceError::TemporarilyUnavailable(format!("Failed to connect to ckBTC minter: {err:?}"))))
    }

    async fn mint_erc20(
        &self,
        eth_address: H160,
        amount: u64,
        base_utxo: Utxo,
    ) -> Result<Erc20MintStatus, Erc20MintError> {
        let mint_order = self
            .prepare_mint_order(eth_address, amount, base_utxo)
            .await?;
        Ok(match self.send_mint_order(mint_order).await {
            Ok(tx_id) => Erc20MintStatus::Minted { amount, tx_id },
            Err(err) => {
                log::warn!("Failed to send mint order: {err:?}");
                Erc20MintStatus::Signed(mint_order)
            }
        })
    }

    async fn prepare_mint_order(
        &self,
        eth_address: H160,
        amount: u64,
        base_utxo: Utxo,
    ) -> Result<SignedMintOrder, Erc20MintError> {
        log::trace!("preparing mint order");

        let sender_chain_id = get_state().borrow().btc_chain_id();
        let sender = Id256::from_evm_address(&eth_address, sender_chain_id);
        let src_token = (&get_state().borrow().ck_btc_ledger()).into();

        let recipient_chain_id = get_state().borrow().erc20_token_id();

        // todo: check if this is correct. Maybe we need to sue txid here?
        let nonce = base_utxo.height;

        let mint_order = MintOrder {
            amount: amount.into(),
            sender,
            src_token,
            recipient: eth_address,
            dst_token: H160::zero(),
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name: get_state().borrow().token_name(),
            symbol: get_state().borrow().token_symbol(),
            decimals: get_state().borrow().decimals(),
        };

        let signer = get_state().borrow().signer().get().clone();
        let signed_mint_order = mint_order
            .encode_and_sign(&signer)
            .await
            .map_err(|err| Erc20MintError::Sign(format!("{err:?}")))?;

        get_state()
            .borrow_mut()
            .push_mint_order(sender, nonce, signed_mint_order.clone());

        log::trace!("Mint order added");

        Ok(signed_mint_order)
    }

    async fn send_mint_order(&self, mint_order: SignedMintOrder) -> Result<H256, Erc20MintError> {
        log::trace!("Sending mint transaction");

        let signer = get_state().borrow().signer().get().clone();
        let sender = signer
            .get_address()
            .await
            .map_err(|err| Erc20MintError::Sign(format!("{err:?}")))?;

        let evm_info = get_state().borrow().get_evm_info();

        let evm_params = get_state()
            .borrow()
            .get_evm_params()
            .clone()
            .ok_or(Erc20MintError::NotInitialized)?;

        let mut tx = minter_contract_utils::bft_bridge_api::mint_transaction(
            sender.0,
            evm_info.bridge_contract.0,
            evm_params.nonce.into(),
            evm_params.gas_price.into(),
            mint_order.to_vec(),
            evm_params.chain_id as _,
        );

        let signature = signer
            .sign_transaction(&(&tx).into())
            .await
            .map_err(|err| Erc20MintError::Sign(format!("{err:?}")))?;

        tx.r = signature.r.0;
        tx.s = signature.s.0;
        tx.v = signature.v.0;
        tx.hash = tx.hash();

        let client = evm_info.link.get_client();
        let id = client
            .send_raw_transaction(tx)
            .await
            .map_err(|err| Erc20MintError::Evm(format!("{err:?}")))?;

        log::trace!("Mint transaction sent");

        Ok(id.into())
    }

    fn schedule_mint(&self, _eth_address: H160) {
        todo!()
    }

    pub fn idl() -> Idl {
        generate_idl!()
    }
}

fn eth_address_to_subaccount(eth_address: &H160) -> Subaccount {
    let mut subaccount = [0; 32];
    subaccount.copy_from_slice(eth_address.0.as_bytes());

    Subaccount(subaccount)
}

impl Metrics for BtcBridge {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

thread_local! {
    pub static STATE: Rc<RefCell<State>> = Rc::default();
}

pub fn get_state() -> Rc<RefCell<State>> {
    STATE.with(|state| state.clone())
}