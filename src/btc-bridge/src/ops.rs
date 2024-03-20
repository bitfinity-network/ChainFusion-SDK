use crate::canister::{eth_address_to_subaccount, get_scheduler};
use crate::ck_btc_interface::{UpdateBalanceArgs, UpdateBalanceError, UtxoStatus};
use crate::interface::{Erc20MintError, Erc20MintStatus};
use crate::scheduler::BtcTask;
use crate::state::State;
use candid::Nat;
use did::{H160, H256};
use eth_signer::sign_strategy::TransactionSigner;
use ic_canister::virtual_canister_call;
use ic_exports::ic_kit::ic;
use ic_exports::icrc_types::icrc1::transfer::{TransferArg, TransferError};
use ic_stable_structures::CellStructure;
use ic_task_scheduler::scheduler::TaskScheduler;
use ic_task_scheduler::task::TaskOptions;
use minter_did::id256::Id256;
use minter_did::order::{MintOrder, SignedMintOrder};
use std::cell::RefCell;
use std::rc::Rc;

pub async fn btc_to_erc20(
    state: Rc<RefCell<State>>,
    eth_address: H160,
) -> Vec<Result<Erc20MintStatus, Erc20MintError>> {
    match request_update_balance(&state, &eth_address).await {
        Ok(minted_utxos) => {
            let mut results = vec![];
            for utxo in minted_utxos {
                let eth_address = eth_address.clone();
                let res = match utxo {
                    UtxoStatus::Minted {
                        minted_amount,
                        utxo,
                        ..
                    } => mint_erc20(&state, eth_address, minted_amount, utxo.height).await,
                    UtxoStatus::ValueTooSmall(_) => Err(Erc20MintError::ValueTooSmall),
                    UtxoStatus::Tainted(utxo) => Err(Erc20MintError::Tainted(utxo)),
                    UtxoStatus::Checked(_) => Err(Erc20MintError::CkBtcMinter(
                        UpdateBalanceError::TemporarilyUnavailable(
                            "KYT check passed, but mint failed. Try again later.".to_string(),
                        ),
                    )),
                };

                results.push(res);
            }

            results
        }
        Err(UpdateBalanceError::NoNewUtxos {
            current_confirmations: None,
            ..
        }) => vec![Err(Erc20MintError::NothingToMint)],
        Err(UpdateBalanceError::NoNewUtxos {
            current_confirmations: Some(curr_confirmations),
            required_confirmations,
            pending_utxos,
        }) => {
            schedule_mint(eth_address);
            vec![Ok(Erc20MintStatus::Scheduled {
                current_confirmations: curr_confirmations,
                required_confirmations,
                pending_utxos,
            })]
        }
        Err(err) => vec![Err(Erc20MintError::CkBtcMinter(err))],
    }
}

async fn request_update_balance(
    state: &RefCell<State>,
    eth_address: &H160,
) -> Result<Vec<UtxoStatus>, UpdateBalanceError> {
    let self_id = ic::id();
    let ck_btc_minter = state.borrow().ck_btc_minter();
    let subaccount = eth_address_to_subaccount(eth_address);

    let args = UpdateBalanceArgs {
        owner: Some(self_id),
        subaccount: Some(subaccount),
    };

    virtual_canister_call!(
        ck_btc_minter,
        "update_balance",
        (args,),
        Result<Vec<UtxoStatus>, UpdateBalanceError>
    )
    .await
    .unwrap_or_else(|err| {
        Err(UpdateBalanceError::TemporarilyUnavailable(format!(
            "Failed to connect to ckBTC minter: {err:?}"
        )))
    })
}

fn schedule_mint(eth_address: H160) {
    let scheduler = get_scheduler();
    let scheduler = scheduler.borrow_mut();
    let task = BtcTask::MintErc20(eth_address);
    let options = TaskOptions::new();
    scheduler.append_task(task.into_scheduled(options));
}

pub async fn mint_erc20(
    state: &RefCell<State>,
    eth_address: H160,
    amount: u64,
    nonce: u32,
) -> Result<Erc20MintStatus, Erc20MintError> {
    let fee = state.borrow().ck_btc_ledger_fee();
    let amount_minus_fee = amount
        .checked_sub(fee)
        .ok_or(Erc20MintError::ValueTooSmall)?;

    let mint_order =
        prepare_mint_order(state, eth_address.clone(), amount_minus_fee, nonce).await?;
    transfer_ckbtc_from_subaccount(state, &eth_address, amount_minus_fee).await?;
    store_mint_order(state, mint_order, &eth_address, nonce);

    Ok(match send_mint_order(state, mint_order).await {
        Ok(tx_id) => Erc20MintStatus::Minted {
            amount: amount_minus_fee,
            tx_id,
        },
        Err(err) => {
            log::warn!("Failed to send mint order: {err:?}");
            Erc20MintStatus::Signed(Box::new(mint_order))
        }
    })
}

async fn transfer_ckbtc_from_subaccount(
    state: &RefCell<State>,
    eth_address: &H160,
    amount: u64,
) -> Result<Nat, TransferError> {
    let (ledger, fee) = {
        let state_ref = state.borrow();
        let ledger = state_ref.ck_btc_ledger();
        let fee = state_ref.ck_btc_ledger_fee();
        (ledger, fee)
    };

    let args = TransferArg {
        from_subaccount: Some(eth_address_to_subaccount(eth_address).0),
        to: ic_exports::icrc_types::icrc1::account::Account {
            owner: ic::id(),
            subaccount: None,
        },
        fee: Some(fee.into()),
        created_at_time: None,
        memo: None,
        amount: amount.into(),
    };

    virtual_canister_call!(ledger, "icrc1_transfer", (args,), Result<Nat, TransferError>)
        .await
        .unwrap_or(Err(TransferError::TemporarilyUnavailable))
}

async fn prepare_mint_order(
    state: &RefCell<State>,
    eth_address: H160,
    amount: u64,
    nonce: u32,
) -> Result<SignedMintOrder, Erc20MintError> {
    log::trace!("preparing mint order");

    let (signer, mint_order) = {
        let state_ref = state.borrow();

        let sender_chain_id = state_ref.btc_chain_id();
        let sender = Id256::from_evm_address(&eth_address, sender_chain_id);
        let src_token = (&state_ref.ck_btc_ledger()).into();

        let recipient_chain_id = state_ref.erc20_chain_id();

        let mint_order = MintOrder {
            amount: amount.into(),
            sender,
            src_token,
            recipient: eth_address,
            dst_token: H160::default(),
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name: state_ref.token_name(),
            symbol: state_ref.token_symbol(),
            decimals: state_ref.decimals(),
        };

        let signer = state_ref.signer().get().clone();

        (signer, mint_order)
    };

    let signed_mint_order = mint_order
        .encode_and_sign(&signer)
        .await
        .map_err(|err| Erc20MintError::Sign(format!("{err:?}")))?;

    Ok(signed_mint_order)
}

fn store_mint_order(
    state: &RefCell<State>,
    signed_mint_order: SignedMintOrder,
    eth_address: &H160,
    nonce: u32,
) {
    let mut state = state.borrow_mut();
    let sender_chain_id = state.btc_chain_id();
    let sender = Id256::from_evm_address(eth_address, sender_chain_id);
    state
        .mint_orders_mut()
        .push(sender, nonce, signed_mint_order);

    log::trace!("Mint order added");
}

async fn send_mint_order(
    state: &RefCell<State>,
    mint_order: SignedMintOrder,
) -> Result<H256, Erc20MintError> {
    log::trace!("Sending mint transaction");

    let signer = state.borrow().signer().get().clone();
    let sender = signer
        .get_address()
        .await
        .map_err(|err| Erc20MintError::Sign(format!("{err:?}")))?;

    let (evm_info, evm_params) = {
        let state = state.borrow();

        let evm_info = state.get_evm_info();
        let evm_params = state
            .get_evm_params()
            .clone()
            .ok_or(Erc20MintError::NotInitialized)?;

        (evm_info, evm_params)
    };

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