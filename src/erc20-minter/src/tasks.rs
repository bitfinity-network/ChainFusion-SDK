use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use did::{H160, U256};
use eth_signer::sign_strategy::TransactionSigner;
use ethers_core::types::{BlockNumber, Log};
use ic_stable_structures::CellStructure;
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::TaskScheduler;
use ic_task_scheduler::task::{ScheduledTask, Task, TaskOptions};
use ic_task_scheduler::SchedulerError;
use jsonrpc_core::Id;
use minter_contract_utils::bft_bridge_api::{self, BridgeEvent, MintedEventData};
use minter_contract_utils::evm_bridge::{BridgeSide, EvmParams};
use minter_contract_utils::operation_store::MinterOperationId;
use minter_contract_utils::query::{self, Query, QueryType, GAS_PRICE_ID, NONCE_ID};
use minter_did::id256::Id256;
use minter_did::order::MintOrder;
use serde::{Deserialize, Serialize};

use crate::canister::{get_operations_store, get_state};
use crate::operation::{OperationPayload, OperationStatus};
use crate::state::State;

/// Task for the ERC-20 bridge
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum BridgeTask {
    InitEvmState(BridgeSide),
    CollectEvmEvents(BridgeSide),
    PrepareMintOrder(MinterOperationId),
    RemoveMintOrder(MintedEventData, BridgeSide),
    SendMintTransaction(MinterOperationId),
}

impl Task for BridgeTask {
    fn execute(
        &self,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), SchedulerError>>>> {
        log::trace!("Running ERC-20 task: {:?}", self);

        let state = get_state();
        match self {
            BridgeTask::InitEvmState(side) => Box::pin(Self::init_evm_state(state, *side)),
            BridgeTask::CollectEvmEvents(side) => {
                let side = *side;
                Box::pin(Self::collect_evm_events(state, scheduler, side))
            }
            BridgeTask::PrepareMintOrder(operation_id) => {
                let operation_id = *operation_id;
                Box::pin(Self::prepare_mint_order(state, scheduler, operation_id))
            }
            BridgeTask::RemoveMintOrder(event_data, sender_side) => {
                let event_data = event_data.clone();
                let sender_side = *sender_side;
                Box::pin(async move { Self::remove_mint_order(event_data, sender_side) })
            }
            BridgeTask::SendMintTransaction(operation_id) => {
                let operation_id = *operation_id;
                Box::pin(Self::send_mint_transaction(state, operation_id))
            }
        }
    }
}

impl BridgeTask {
    pub fn into_scheduled(self, options: TaskOptions) -> ScheduledTask<Self> {
        ScheduledTask::with_options(self, options)
    }

    pub async fn init_evm_state(
        state: Rc<RefCell<State>>,
        side: BridgeSide,
    ) -> Result<(), SchedulerError> {
        let client = state
            .borrow()
            .config
            .get_evm_info(side)
            .link
            .get_json_rpc_client();

        let address = {
            let signer = state.borrow().signer.get().clone();
            signer.get_address().await.into_scheduler_result()?
        };

        let evm_params = EvmParams::query(client, address)
            .await
            .into_scheduler_result()?;

        state
            .borrow_mut()
            .config
            .update_evm_params(|old| *old = evm_params, side);

        log::trace!("evm state initialized for side {:?}", side);

        Ok(())
    }

    async fn collect_evm_events(
        state: Rc<RefCell<State>>,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
        side: BridgeSide,
    ) -> Result<(), SchedulerError> {
        log::trace!("collecting evm events: side: {side:?}");

        let evm_info = state.borrow().config.get_evm_info(side);
        let Some(params) = evm_info.params else {
            log::warn!("no evm params for side {side} found");
            return Self::init_evm_state(state, side).await;
        };

        let bft_bridge = state
            .borrow()
            .config
            .get_bft_bridge_contract(side)
            .ok_or_else(|| {
                SchedulerError::TaskExecutionFailed("no bft bridge contract set".into())
            })?;

        let client = evm_info.link.get_json_rpc_client();
        let last_block = client.get_block_number().await.into_scheduler_result()?;

        let logs = BridgeEvent::collect_logs(&client, params.next_block, last_block, bft_bridge.0)
            .await
            .into_scheduler_result()?;

        log::debug!("got logs from side {side}: {logs:?}");

        state
            .borrow_mut()
            .config
            .update_evm_params(|params| params.next_block = last_block + 1, side);

        log::trace!("appending logs to tasks: {side:?}: {logs:?}");

        scheduler.append_tasks(
            logs.into_iter()
                .filter_map(|l| Self::task_by_log(l, side))
                .collect(),
        );

        // Update the EVM params
        Self::update_evm_params(state.clone(), side).await?;

        Ok(())
    }

    async fn prepare_mint_order(
        state: Rc<RefCell<State>>,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
        operation_id: MinterOperationId,
    ) -> Result<(), SchedulerError> {
        let mut operation_store = get_operations_store();
        let Some(operation) = operation_store.get(operation_id) else {
            return Err(SchedulerError::TaskExecutionFailed(format!(
                "Operation {operation_id} is not found in the operation store."
            )));
        };

        let burn_side = operation.side;
        let OperationStatus::Scheduled(burn_event) = operation.status else {
            return Err(SchedulerError::TaskExecutionFailed(format!("Operation {operation_id} was expected to be in `Scheduled` state, but found: {operation:?}")));
        };

        log::trace!("preparing mint order: {burn_event:?}");

        let burn_evm_params = state
            .borrow()
            .config
            .get_evm_params(burn_side)
            .into_scheduler_result()?;

        let mint_evm_params = state
            .borrow()
            .config
            .get_evm_params(burn_side.other())
            .into_scheduler_result()?;

        let recipient = Id256::from_slice(&burn_event.recipient_id)
            .and_then(|id| id.to_evm_address().ok())
            .ok_or_else(|| {
                log::error!("failed to decode recipient data: {burn_event:?}");
                SchedulerError::TaskExecutionFailed("failed to decode recipient data".into())
            })?
            .1;

        let dst_token = Id256::from_slice(&burn_event.to_token)
            .and_then(|id| id.to_evm_address().ok())
            .unwrap_or_default()
            .1;

        let sender_chain_id = burn_evm_params.chain_id as u32;
        let recipient_chain_id = mint_evm_params.chain_id as u32;

        let sender = Id256::from_evm_address(&burn_event.sender, sender_chain_id);
        let src_token = Id256::from_evm_address(&burn_event.from_erc20, sender_chain_id);

        fn to_array<const N: usize>(data: &[u8]) -> Result<[u8; N], SchedulerError> {
            data.try_into().into_scheduler_result()
        }

        let nonce = burn_event.operation_id;
        let amount = burn_event.amount;

        let mint_order = MintOrder {
            amount: amount.clone(),
            sender,
            src_token,
            recipient,
            dst_token,
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name: to_array(&burn_event.name)?,
            symbol: to_array(&burn_event.symbol)?,
            decimals: burn_event.decimals,
            approve_spender: H160::zero(),
            approve_amount: U256::zero(),
            fee_payer: burn_event.sender,
        };

        let signer = state.borrow().signer.get().clone();
        let signed_mint_order = mint_order
            .encode_and_sign(&signer)
            .await
            .into_scheduler_result()?;

        operation_store.update(
            operation_id,
            OperationPayload {
                side: burn_side,
                status: OperationStatus::MintOrderSigned {
                    token_id: src_token,
                    amount,
                    signed_mint_order: Box::new(signed_mint_order),
                },
            },
        );

        // Update the EVM params
        Self::update_evm_params(state.clone(), burn_side).await?;

        let options = TaskOptions::default();
        scheduler
            .append_task(BridgeTask::SendMintTransaction(operation_id).into_scheduled(options));

        log::trace!("Mint order added");

        Ok(())
    }

    fn task_by_log(log: Log, sender_side: BridgeSide) -> Option<ScheduledTask<BridgeTask>> {
        log::trace!("creating task from the log: {log:?}");

        const TASK_RETRY_DELAY_SECS: u32 = 5;

        let options = TaskOptions::default()
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: TASK_RETRY_DELAY_SECS,
            })
            .with_max_retries_policy(u32::MAX);

        match BridgeEvent::from_log(log).into_scheduler_result() {
            Ok(BridgeEvent::Burnt(burnt)) => {
                log::debug!("Adding PrepareMintOrder task");
                let operation_id = get_operations_store().new_operation(
                    burnt.sender.clone(),
                    OperationPayload::new(sender_side.other(), burnt),
                );
                let mint_order_task = BridgeTask::PrepareMintOrder(operation_id);
                return Some(mint_order_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Minted(minted)) => {
                log::debug!("Adding RemoveMintOrder task");
                let remove_mint_order_task = BridgeTask::RemoveMintOrder(minted, sender_side);
                return Some(remove_mint_order_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Notify(_)) => todo!(),
            Err(e) => log::warn!("collected log is incompatible with expected events: {e}"),
        }

        None
    }

    fn remove_mint_order(
        minted_event: MintedEventData,
        sender_side: BridgeSide,
    ) -> Result<(), SchedulerError> {
        let wallet_id = match sender_side {
            BridgeSide::Base => minted_event.recipient,
            BridgeSide::Wrapped => {
                Id256::from_slice(&minted_event.sender_id)
                    .ok_or_else(|| {
                        SchedulerError::TaskExecutionFailed(
                            "failed to decode sender id256 from minted event".into(),
                        )
                    })?
                    .to_evm_address()
                    .map_err(|_| {
                        SchedulerError::TaskExecutionFailed(
                            "sender id was not an EVM address".into(),
                        )
                    })?
                    .1
            }
        };

        let mut operation_store = get_operations_store();
        let nonce = minted_event.nonce;
        let Some((operation_id, operation_state)) = operation_store
            .get_for_address(&wallet_id, None, None)
            .into_iter()
            .find(|(operation_id, _)| operation_id.nonce() == nonce)
        else {
            log::error!("operation with nonce {nonce} not found");
            return Err(SchedulerError::TaskExecutionFailed(format!(
                "operation with nonce {nonce} not found"
            )));
        };

        let src_token = Id256::from_slice(&minted_event.from_token).ok_or_else(|| {
            SchedulerError::TaskExecutionFailed(
                "failed to decode token id256 from minted event".into(),
            )
        })?;

        if let OperationStatus::MintOrderSent {
            token_id,
            amount,
            tx_id,
            ..
        } = operation_state.status
        {
            if token_id == src_token {
                operation_store.update(
                    operation_id,
                    OperationPayload {
                        side: operation_state.side,
                        status: OperationStatus::Minted {
                            amount,
                            token_id,
                            tx_id,
                        },
                    },
                );

                log::trace!("Mint order removed");
            } else {
                log::warn!("Operation {operation_id} was created for token id {token_id:?} but the mint event is emitted by {src_token:?}.");
            }
        } else {
            log::error!("Operation {operation_id} was expected to be in `MintOrderSent` state, but was found: {operation_state:?}");
        }

        Ok(())
    }

    async fn send_mint_transaction(
        state: Rc<RefCell<State>>,
        operation_id: MinterOperationId,
    ) -> Result<(), SchedulerError> {
        let mut operation_store = get_operations_store();
        let Some(operation) = operation_store.get(operation_id) else {
            return Err(SchedulerError::TaskExecutionFailed(format!(
                "Operation {operation_id} is not found in the operation store."
            )));
        };

        let side = operation.side;
        let OperationStatus::MintOrderSigned {
            token_id,
            amount,
            signed_mint_order,
        } = operation.status
        else {
            return Err(SchedulerError::TaskExecutionFailed(format!("Operation {operation_id} was expected to be in `MintOrderSigned` state, but found: {operation:?}")));
        };

        log::trace!("Sending mint transaction");

        let signer = state.borrow().signer.get().clone();
        let sender = signer.get_address().await.into_scheduler_result()?;

        let evm_info = state.borrow().config.get_evm_info(side);

        let evm_params = state
            .borrow()
            .config
            .get_evm_params(side)
            .into_scheduler_result()?;

        let bft_bridge = &state
            .borrow()
            .config
            .get_bft_bridge_contract(side)
            .ok_or_else(|| {
                log::warn!("failed to send mint transaction: bft bridge is not configured");
                SchedulerError::TaskExecutionFailed("bft bridge is not configured".into())
            })?;

        let client = evm_info.link.get_json_rpc_client();
        let nonce = client
            .get_transaction_count(sender.0, BlockNumber::Latest)
            .await
            .into_scheduler_result()?;

        let mut tx = bft_bridge_api::mint_transaction(
            sender.0,
            bft_bridge.0,
            nonce.into(),
            evm_params.gas_price.into(),
            &signed_mint_order.0,
            evm_params.chain_id as _,
        );

        let signature = signer
            .sign_transaction(&(&tx).into())
            .await
            .into_scheduler_result()?;
        tx.r = signature.r.0;
        tx.s = signature.s.0;
        tx.v = signature.v.0;
        tx.hash = tx.hash();

        let client = evm_info.link.get_json_rpc_client();
        let tx_id = client
            .send_raw_transaction(tx)
            .await
            .into_scheduler_result()?;

        operation_store.update(
            operation_id,
            OperationPayload {
                side,
                status: OperationStatus::MintOrderSent {
                    token_id,
                    amount,
                    signed_mint_order,
                    tx_id: tx_id.into(),
                },
            },
        );

        log::trace!("Mint transaction sent. Operation id: {operation_id}. Tx id: {tx_id}");

        Ok(())
    }

    pub async fn update_evm_params(
        state: Rc<RefCell<State>>,
        side: BridgeSide,
    ) -> Result<(), SchedulerError> {
        let evm_info = state.borrow().config.get_evm_info(side);

        let initial_params = state
            .borrow()
            .config
            .get_evm_params(side)
            .into_scheduler_result()?;
        let address = {
            let signer = state.borrow().signer.get().clone();
            signer.get_address().await.into_scheduler_result()?
        };
        // Update the EvmParams
        log::trace!("updating evm params");
        let responses = query::batch_query(
            &evm_info.link.get_json_rpc_client(),
            &[
                QueryType::Nonce {
                    address: address.into(),
                },
                QueryType::GasPrice,
            ],
        )
        .await
        .into_scheduler_result()?;

        let nonce: U256 = responses
            .get_value_by_id(Id::Str(NONCE_ID.into()))
            .into_scheduler_result()?;
        let gas_price: U256 = responses
            .get_value_by_id(Id::Str(GAS_PRICE_ID.into()))
            .into_scheduler_result()?;

        let params = EvmParams {
            nonce: nonce.0.as_u64(),
            gas_price,
            ..initial_params
        };

        state
            .borrow_mut()
            .config
            .update_evm_params(|p| *p = params, side);
        log::trace!("evm params updated");

        Ok(())
    }
}

trait IntoSchedulerError {
    type Success;

    fn into_scheduler_result(self) -> Result<Self::Success, SchedulerError>;
}

impl<T, E: ToString> IntoSchedulerError for Result<T, E> {
    type Success = T;

    fn into_scheduler_result(self) -> Result<Self::Success, SchedulerError> {
        self.map_err(|e| SchedulerError::TaskExecutionFailed(e.to_string()))
    }
}
