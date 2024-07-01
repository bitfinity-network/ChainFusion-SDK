use std::future::Future;
use std::pin::Pin;

use did::{H160, U256};
use eth_signer::sign_strategy::TransactionSigner;
use ethers_core::types::Log;
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{CellStructure, StableBTreeMap, VirtualMemory};
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::{Scheduler, TaskScheduler};
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, Task, TaskOptions};
use ic_task_scheduler::SchedulerError;
use jsonrpc_core::Id;
use minter_contract_utils::bft_bridge_api::{BridgeEvent, BurntEventData, MintedEventData};
use minter_contract_utils::evm_bridge::EvmParams;
use minter_contract_utils::query::{self, Query, QueryType, GAS_PRICE_ID, NONCE_ID};
use minter_did::id256::Id256;
use serde::{Deserialize, Serialize};

use crate::canister::get_state;

pub type TasksStorage =
    StableBTreeMap<u32, InnerScheduledTask<BtcTask>, VirtualMemory<DefaultMemoryImpl>>;

pub type PersistentScheduler = Scheduler<BtcTask, TasksStorage>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum BtcTask {
    InitEvmState,
    CollectEvmEvents,
    RemoveMintOrder(MintedEventData),
    MintBtc(BurntEventData),
    MintErc20(H160),
}

impl BtcTask {
    pub fn into_scheduled(self, options: TaskOptions) -> ScheduledTask<Self> {
        ScheduledTask::with_options(self, options)
    }

    pub async fn init_evm_state() -> Result<(), SchedulerError> {
        let state = get_state();
        let client = state.borrow().get_evm_info().link.get_json_rpc_client();
        let address = {
            let signer = state.borrow().signer().get().clone();
            signer.get_address().await.into_scheduler_result()?
        };

        let evm_params = EvmParams::query(client, address)
            .await
            .into_scheduler_result()?;

        state
            .borrow_mut()
            .update_evm_params(|old| *old = Some(evm_params));

        log::trace!("Evm state is initialized");

        Ok(())
    }

    async fn collect_evm_events(
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Result<(), SchedulerError> {
        log::trace!("collecting evm events");

        let state = get_state();
        let evm_info = state.borrow().get_evm_info();
        let Some(params) = evm_info.params else {
            log::warn!("no evm params initialized");
            return Ok(());
        };

        let client = evm_info.link.get_json_rpc_client();
        let last_block = client.get_block_number().await.into_scheduler_result()?;

        let logs = BridgeEvent::collect_logs(
            &client,
            params.next_block,
            last_block,
            evm_info.bridge_contract.0,
        )
        .await
        .into_scheduler_result()?;

        log::debug!("got {} logs from evm", logs.len());

        if logs.is_empty() {
            return Ok(());
        }

        state.borrow_mut().update_evm_params(|to_update| {
            *to_update = Some(EvmParams {
                next_block: last_block + 1,
                ..params
            })
        });

        log::trace!("appending logs to tasks");

        scheduler.append_tasks(logs.into_iter().filter_map(Self::task_by_log).collect());

        Self::update_evm_params().await?;

        Ok(())
    }

    fn task_by_log(log: Log) -> Option<ScheduledTask<BtcTask>> {
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
                let mint_order_task = BtcTask::MintBtc(burnt);
                return Some(mint_order_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Minted(minted)) => {
                log::debug!("Adding RemoveMintOrder task");
                let remove_mint_order_task = BtcTask::RemoveMintOrder(minted);
                return Some(remove_mint_order_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Notify(_)) => return None,
            Err(e) => log::warn!("collected log is incompatible with expected events: {e}"),
        }

        None
    }

    fn remove_mint_order(minted_event: MintedEventData) -> Result<(), SchedulerError> {
        let state = get_state();
        let sender_id = Id256::from_slice(&minted_event.sender_id).ok_or_else(|| {
            SchedulerError::TaskExecutionFailed(
                "failed to decode sender id256 from minted event".into(),
            )
        })?;

        state
            .borrow_mut()
            .mint_orders_mut()
            .remove(sender_id, minted_event.nonce);

        log::trace!("Mint order removed");

        Ok(())
    }

    pub async fn update_evm_params() -> Result<(), SchedulerError> {
        let state = get_state();
        let evm_info = state.borrow().get_evm_info();

        let Some(initial_params) = evm_info.params else {
            log::warn!("no evm params initialized");
            return Ok(());
        };

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
            .update_evm_params(|old| *old = Some(params));

        log::trace!("evm params updated");

        Ok(())
    }
}

impl Task for BtcTask {
    fn execute(
        &self,
        task_scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), SchedulerError>>>> {
        match self {
            BtcTask::InitEvmState => Box::pin(Self::init_evm_state()),
            BtcTask::CollectEvmEvents => Box::pin(Self::collect_evm_events(task_scheduler)),
            BtcTask::RemoveMintOrder(data) => {
                let data = data.clone();
                Box::pin(async move { Self::remove_mint_order(data) })
            }
            BtcTask::MintErc20(address) => {
                let address = address.clone();
                Box::pin(async move {
                    // Update the EvmParams
                    Self::update_evm_params().await?;

                    let result = crate::ops::btc_to_erc20(get_state(), address).await;

                    log::info!("ERC20 mint result from scheduler: {result:?}");

                    Ok(())
                })
            }
            BtcTask::MintBtc(BurntEventData {
                operation_id,
                recipient_id,
                amount,
                ..
            }) => {
                log::info!("ERC20 burn event received");

                let amount = amount.0.as_u64();
                let operation_id = *operation_id;

                let Ok(address) = String::from_utf8(recipient_id.clone()) else {
                    return Box::pin(futures::future::err(SchedulerError::TaskExecutionFailed(
                        "Failed to decode recipient address".to_string(),
                    )));
                };

                Box::pin(async move {
                    let result =
                        crate::ops::burn_ckbtc(&get_state(), operation_id, &address, amount)
                            .await
                            .map_err(|err| {
                                SchedulerError::TaskExecutionFailed(format!("{err:?}"))
                            })?;

                    log::info!(
                        "Created withdrawal transaction at block {}",
                        result.block_index
                    );

                    Ok(())
                })
            }
        }
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
