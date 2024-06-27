use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use candid::{CandidType, Decode, Nat, Principal};
use did::{H160, U256};
use eth_signer::sign_strategy::TransactionSigner;
use ethers_core::types::{BlockNumber, Log};
use ic_exports::ic_kit::RejectionCode;
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::TaskScheduler;
use ic_task_scheduler::task::{ScheduledTask, Task, TaskOptions};
use ic_task_scheduler::SchedulerError;
use icrc_client::account::Account;
use icrc_client::transfer::TransferError;
use jsonrpc_core::Id;
use minter_contract_utils::bft_bridge_api::{self, BridgeEvent, MintedEventData};
use minter_contract_utils::evm_bridge::EvmParams;
use minter_contract_utils::evm_link::address_to_icrc_subaccount;
use minter_contract_utils::operation_store::MinterOperationId;
use minter_contract_utils::query::{self, Query, QueryType, GAS_PRICE_ID, NONCE_ID};
use minter_did::error::Error;
use minter_did::id256::Id256;
use minter_did::order::{self, MintOrder};
use minter_did::reason::Icrc2Burn;
use serde::{Deserialize, Serialize};

use crate::canister::get_operations_store;
use crate::constant::IC_CHAIN_ID;
use crate::operation::{DepositOperationState, OperationState, WithdrawalOperationState};
use crate::state::State;
use crate::tokens::icrc2::Success;
use crate::tokens::{icrc1, icrc2};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum BridgeTask {
    InitEvmInfo,
    CollectEvmEvents,
    BurnIcrc2Tokens(MinterOperationId),
    PrepareMintOrder(MinterOperationId),
    RemoveMintOrder(MintedEventData),
    SendMintTransaction(MinterOperationId),
    MintIcrc2Tokens(MinterOperationId),
}

impl Task for BridgeTask {
    fn execute(
        &self,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Pin<Box<dyn Future<Output = Result<(), SchedulerError>>>> {
        let state = crate::canister::get_state();
        match self {
            BridgeTask::InitEvmInfo => Box::pin(Self::init_evm_info(state)),
            BridgeTask::CollectEvmEvents => Box::pin(Self::collect_evm_events(state, scheduler)),
            BridgeTask::BurnIcrc2Tokens(operation_id) => {
                Box::pin(Self::burn_icrc2_tokens(scheduler, *operation_id))
            }
            BridgeTask::PrepareMintOrder(operation_id) => {
                Box::pin(Self::prepare_mint_order(state, scheduler, *operation_id))
            }
            BridgeTask::RemoveMintOrder(data) => {
                let data = data.clone();
                Box::pin(async move { Self::remove_mint_order(data) })
            }
            BridgeTask::SendMintTransaction(operation_id) => {
                Box::pin(Self::send_mint_transaction(state, *operation_id))
            }
            BridgeTask::MintIcrc2Tokens(operation_id) => {
                Box::pin(Self::mint_icrc2(*operation_id, scheduler))
            }
        }
    }
}

impl BridgeTask {
    pub fn into_scheduled(self, options: TaskOptions) -> ScheduledTask<Self> {
        ScheduledTask::with_options(self, options)
    }

    pub(crate) async fn init_evm_info(state: Rc<RefCell<State>>) -> Result<(), SchedulerError> {
        log::trace!("evm info initialization started");

        let client = state.borrow().config.get_evm_client();
        let address = {
            let signer = state.borrow().signer.get_transaction_signer();
            signer.get_address().await.into_scheduler_result()?
        };

        let evm_params = EvmParams::query(client, address)
            .await
            .into_scheduler_result()?;

        state
            .borrow_mut()
            .config
            .update_evm_params(|p| *p = evm_params);

        log::trace!("evm parameters initialized");

        Ok(())
    }

    async fn collect_evm_events(
        state: Rc<RefCell<State>>,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Result<(), SchedulerError> {
        log::trace!("collecting evm events");

        let client = state.borrow().config.get_evm_client();
        let Some(params) = state.borrow().config.get_evm_params() else {
            log::warn!("no evm parameters set, unable to collect events");
            return Err(SchedulerError::TaskExecutionFailed(
                "no evm parameters set".into(),
            ));
        };
        let Some(bridge_contract) = state.borrow().config.get_bft_bridge_contract() else {
            log::warn!("no bft bridge contract set, unable to collect events");
            return Err(SchedulerError::TaskExecutionFailed(
                "no bft bridge contract set".into(),
            ));
        };

        let logs = BridgeEvent::collect_logs(
            &client,
            params.next_block.into(),
            BlockNumber::Safe,
            bridge_contract.0,
        )
        .await
        .into_scheduler_result()?;

        log::debug!("got evm logs: {logs:?}");

        // Filter out logs that do not have block number.
        // Such logs are produced when the block is not finalized yet.
        let last_log = logs.iter().take_while(|l| l.block_number.is_some()).last();
        if let Some(last_log) = last_log {
            let next_block_number = last_log.block_number.unwrap().as_u64() + 1;
            state
                .borrow_mut()
                .config
                .update_evm_params(|params| params.next_block = next_block_number);
        };

        log::trace!("appending logs to tasks: {logs:?}");

        scheduler.append_tasks(logs.into_iter().filter_map(Self::task_by_log).collect());

        // Update EVM params
        Self::update_evm_params(state.clone()).await?;

        Ok(())
    }

    pub async fn burn_icrc2_tokens(
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
        operation_id: MinterOperationId,
    ) -> Result<(), SchedulerError> {
        let mut operation_store = get_operations_store();
        let operation_state = operation_store.get(operation_id);
        let Some(OperationState::Deposit(DepositOperationState::Scheduled(reason))) =
            operation_state
        else {
            log::error!(
                "deposit request was in incorrect state: {:?}",
                operation_state
            );
            return Ok(());
        };

        let caller_account = Account {
            owner: reason.sender,
            subaccount: reason.from_subaccount,
        };

        let token_info = icrc1::query_token_info_or_read_from_cache(reason.icrc2_token_principal)
            .await
            .ok_or(Error::InvalidBurnOperation(
                "failed to get token info".into(),
            ))
            .into_scheduler_result()?;

        let name = order::fit_str_to_array(&token_info.name);
        let symbol = order::fit_str_to_array(&token_info.symbol);

        let spender_subaccount = address_to_icrc_subaccount(&reason.recipient_address.0);
        icrc2::burn(
            reason.icrc2_token_principal,
            caller_account,
            Some(spender_subaccount),
            (&reason.amount).into(),
            true,
        )
        .await
        .into_scheduler_result()?;

        let nonce = operation_id.nonce();
        let burn_data = BurntIcrc2Data {
            sender: reason.sender,
            amount: reason.amount,
            operation_id: nonce,
            name,
            symbol,
            decimals: token_info.decimals,
            src_token: reason.icrc2_token_principal,
            recipient_address: reason.recipient_address,
            fee_payer: reason.fee_payer,
        };
        operation_store.update(
            operation_id,
            OperationState::Deposit(DepositOperationState::Icrc2Burned(burn_data)),
        );

        let options = TaskOptions::default()
            .with_backoff_policy(BackoffPolicy::Fixed { secs: 4 })
            .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite);

        scheduler.append_task(BridgeTask::PrepareMintOrder(operation_id).into_scheduled(options));

        Ok(())
    }

    async fn prepare_mint_order(
        state: Rc<RefCell<State>>,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
        operation_id: MinterOperationId,
    ) -> Result<(), SchedulerError> {
        let mut operation_store = get_operations_store();
        let operation_state = operation_store.get(operation_id);
        let (burnt_data, is_deposit) = match operation_state {
            Some(OperationState::Deposit(DepositOperationState::Icrc2Burned(burnt_data))) => {
                (burnt_data, true)
            }
            Some(OperationState::Withdrawal(WithdrawalOperationState::RefundScheduled(
                burnt_data,
            ))) => (burnt_data, false),
            _ => {
                log::error!(
                    "deposit request was in incorrect state: {:?}",
                    operation_state
                );
                return Ok(());
            }
        };

        log::trace!("preparing mint order. Is deposit: {is_deposit}: {burnt_data:?}");

        let Some(evm_params) = state.borrow().config.get_evm_params() else {
            log::warn!("no evm parameters set, unable to prepare mint order");
            return Err(SchedulerError::TaskExecutionFailed(
                "no evm parameters set".into(),
            ));
        };

        let sender_chain_id = IC_CHAIN_ID;
        let recipient_chain_id = evm_params.chain_id as u32;

        let sender = Id256::from(&burnt_data.sender);
        let src_token = Id256::from(&burnt_data.src_token);

        let nonce = burnt_data.operation_id;

        // If there is no fee payer, user should send mint tx by himself.
        let fee_payer = burnt_data.fee_payer.unwrap_or_default();
        let should_send_mint_tx = fee_payer != H160::zero();

        let mint_order = MintOrder {
            amount: burnt_data.amount,
            sender,
            src_token,
            recipient: burnt_data.recipient_address,
            dst_token: H160::default(), // will be selected in the contract.
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name: burnt_data.name,
            symbol: burnt_data.symbol,
            decimals: burnt_data.decimals,
            approve_spender: Default::default(),
            approve_amount: Default::default(),
            fee_payer,
        };

        log::debug!("PREPARED MINT ORDER: {:?}", mint_order);

        let signer = state.borrow().signer.get_transaction_signer();
        let signed_mint_order = mint_order
            .encode_and_sign(&signer)
            .await
            .into_scheduler_result()?;

        if is_deposit {
            operation_store.update(
                operation_id,
                OperationState::Deposit(DepositOperationState::MintOrderSigned {
                    token_id: src_token,
                    amount: mint_order.amount,
                    signed_mint_order: Box::new(signed_mint_order),
                }),
            );
        } else {
            operation_store.update(
                operation_id,
                OperationState::Withdrawal(WithdrawalOperationState::RefundMintOrderSigned {
                    token_id: src_token,
                    amount: mint_order.amount,
                    signed_mint_order: Box::new(signed_mint_order),
                }),
            );
        }

        if should_send_mint_tx {
            // Update EVM params before sending the transaction.
            Self::update_evm_params(state.clone()).await?;

            let options = TaskOptions::default();
            scheduler
                .append_task(BridgeTask::SendMintTransaction(operation_id).into_scheduled(options));
        }

        log::trace!("Mint order added");

        Ok(())
    }

    fn task_by_log(log: Log) -> Option<ScheduledTask<BridgeTask>> {
        log::trace!("creating task from the log: {log:?}");

        const TASK_RETRY_DELAY_SECS: u32 = 5;

        let options = TaskOptions::default()
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: TASK_RETRY_DELAY_SECS,
            })
            .with_max_retries_policy(u32::MAX);

        match BridgeEvent::from_log(log).into_scheduler_result() {
            Ok(BridgeEvent::Burnt(burnt)) => {
                log::debug!("Adding MintIcrc2 task");
                let operation_id = get_operations_store()
                    .new_operation(burnt.sender.clone(), OperationState::new_withdrawal(burnt));
                let mint_icrc2_task = BridgeTask::MintIcrc2Tokens(operation_id);
                return Some(mint_icrc2_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Minted(minted)) => {
                log::debug!("Adding RemoveMintOrder task");
                let remove_mint_order_task = BridgeTask::RemoveMintOrder(minted);
                return Some(remove_mint_order_task.into_scheduled(options));
            }
            Ok(BridgeEvent::Notify(notification)) => {
                log::debug!("Adding BurnIcrc2 task");
                let icrc_burn = match Decode!(&notification.user_data, Icrc2Burn) {
                    Ok(icrc_burn) => icrc_burn,
                    Err(e) => {
                        log::warn!("failed to decode BftBridge notification into Icrc2Burn: {e}");
                        return None;
                    }
                };
                let operation_id = get_operations_store().new_operation(
                    icrc_burn.recipient_address.clone(),
                    OperationState::new_deposit(icrc_burn),
                );
                let icrc_burn_task = BridgeTask::BurnIcrc2Tokens(operation_id);
                return Some(icrc_burn_task.into_scheduled(options));
            }
            Err(e) => log::warn!("collected log is incompatible with expected events: {e}"),
        }

        None
    }

    fn remove_mint_order(minted_event: MintedEventData) -> Result<(), SchedulerError> {
        log::trace!("mint order removing");

        let src_token = Id256::from_slice(&minted_event.from_token).ok_or_else(|| {
            log::error!("failed to decode token id256 from minted event",);
            SchedulerError::TaskExecutionFailed(
                "failed to decode token id256 from minted event".into(),
            )
        })?;

        let mut operation_store = get_operations_store();
        let nonce = minted_event.nonce;
        let Some((operation_id, operation_state)) = operation_store
            .get_for_address(&minted_event.recipient)
            .into_iter()
            .find(|(operation_id, _)| operation_id.nonce() == nonce)
        else {
            log::error!("operation with nonce {nonce} not found");
            return Err(SchedulerError::TaskExecutionFailed(format!(
                "operation with nonce {nonce} not found"
            )));
        };

        if matches!(operation_state, OperationState::Deposit(_)) {
            operation_store.update(
                operation_id,
                OperationState::Deposit(DepositOperationState::Minted {
                    token_id: src_token,
                    amount: minted_event.amount,
                }),
            );
        } else {
            operation_store.update(
                operation_id,
                OperationState::Withdrawal(WithdrawalOperationState::RefundMinted {
                    token_id: src_token,
                    amount: minted_event.amount,
                }),
            );
        }

        log::trace!("Mint order removed");

        Ok(())
    }

    async fn send_mint_transaction(
        state: Rc<RefCell<State>>,
        operation_id: MinterOperationId,
    ) -> Result<(), SchedulerError> {
        log::trace!("Sending mint transaction");

        let mut operation_store = get_operations_store();
        let operation_state = operation_store.get(operation_id);
        let Some(OperationState::Deposit(DepositOperationState::MintOrderSigned {
            signed_mint_order,
            amount,
            token_id,
        })) = operation_state
        else {
            log::error!(
                "deposit request was in incorrect state: {:?}",
                operation_state
            );
            return Ok(());
        };

        let signer = state.borrow().signer.get_transaction_signer();
        let sender = signer.get_address().await.into_scheduler_result()?;
        let Some(bridge_contract) = state.borrow().config.get_bft_bridge_contract() else {
            log::warn!("Bridge contract is not set");
            return Err(SchedulerError::TaskExecutionFailed(
                "Bridge contract is not set".into(),
            ));
        };
        let Some(evm_params) = state.borrow().config.get_evm_params() else {
            log::warn!("No evm parameters set");
            return Err(SchedulerError::TaskExecutionFailed(
                "No evm parameters set".into(),
            ));
        };

        let mut tx = bft_bridge_api::mint_transaction(
            sender.0,
            bridge_contract.0,
            evm_params.nonce.into(),
            evm_params.gas_price.clone().into(),
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

        let client = state.borrow().config.get_evm_client();
        let tx_id = client
            .send_raw_transaction(tx)
            .await
            .into_scheduler_result()?;

        operation_store.update(
            operation_id,
            OperationState::Deposit(DepositOperationState::MintOrderSent {
                token_id,
                amount,
                signed_mint_order,
                tx_id: tx_id.into(),
            }),
        );

        log::trace!("Mint transaction sent: {tx_id}");

        Ok(())
    }

    async fn mint_icrc2(
        operation_id: MinterOperationId,
        scheduler: Box<dyn 'static + TaskScheduler<Self>>,
    ) -> Result<(), SchedulerError> {
        log::trace!("Minting Icrc2 tokens");

        let mut operation_store = get_operations_store();
        let operation_state = operation_store.get(operation_id);
        let Some(OperationState::Withdrawal(WithdrawalOperationState::Scheduled(burnt_event))) =
            operation_state
        else {
            log::error!(
                "deposit request was in incorrect state: {:?}",
                operation_state
            );
            return Ok(());
        };

        let Some(to_token) =
            Id256::from_slice(&burnt_event.to_token).and_then(|id| id.try_into().ok())
        else {
            log::warn!("Failed to decode token id256 from erc20 minted event");
            return Err(SchedulerError::TaskExecutionFailed(
                "Failed to decode token id256 from erc20 minted event".into(),
            ));
        };

        let Some(recipient) =
            Id256::from_slice(&burnt_event.recipient_id).and_then(|id| id.try_into().ok())
        else {
            log::warn!("Failed to decode recipient id from minted event");
            return Err(SchedulerError::TaskExecutionFailed(
                "Failed to decode recipient id from minted event".into(),
            ));
        };

        // Transfer icrc2 tokens to the recipient.
        let amount = Nat::from(&burnt_event.amount);

        let mint_result = icrc2::mint(to_token, recipient, amount.clone(), true).await;

        match mint_result {
            Ok(Success { tx_id, amount }) => {
                operation_store.update(
                    operation_id,
                    OperationState::Withdrawal(WithdrawalOperationState::Transferred {
                        token: to_token,
                        recipient: recipient.into(),
                        amount,
                        tx_id,
                    }),
                );

                log::trace!("Finished icrc2 mint to principal: {}", recipient);
                Ok(())
            }
            Err(
                e @ Error::Icrc2TransferError(TransferError::TooOld)
                | e @ Error::Icrc2TransferError(TransferError::CreatedInFuture { .. })
                | e @ Error::Icrc2TransferError(TransferError::TemporarilyUnavailable)
                | e @ Error::Icrc2TransferError(TransferError::GenericError { .. })
                | e @ Error::InterCanisterCallFailed(RejectionCode::SysTransient, _),
            ) => {
                log::warn!("Failed to perform icrc token mint due to: {e}. Retrying...");
                Err(SchedulerError::TaskExecutionFailed(e.to_string()))
            }
            Err(e) => {
                log::warn!(
                    "Impossible to mint icrc token due to: {e}. Preparing refund MintOrder..."
                );

                // If we pass zero name or symbol, it will not be applied.
                let name = burnt_event.name.try_into().unwrap_or_default();
                let symbol = burnt_event.symbol.try_into().unwrap_or_default();
                let burnt_data = BurntIcrc2Data {
                    sender: recipient,
                    amount: burnt_event.amount,
                    src_token: to_token,
                    recipient_address: burnt_event.sender,
                    operation_id: operation_id.nonce(),
                    name,
                    symbol,
                    decimals: burnt_event.decimals,
                    fee_payer: None,
                };

                operation_store.update(
                    operation_id,
                    OperationState::Withdrawal(WithdrawalOperationState::RefundScheduled(
                        burnt_data,
                    )),
                );

                let task = Self::PrepareMintOrder(operation_id);
                let options = TaskOptions::default()
                    .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite)
                    .with_backoff_policy(BackoffPolicy::Exponential {
                        secs: 1,
                        multiplier: 4,
                    });

                let task_id = scheduler.append_task(task.into_scheduled(options));
                log::trace!("Appending refund mint order task#{task_id}.");

                Ok(())
            }
        }
    }

    pub(crate) async fn update_evm_params(state: Rc<RefCell<State>>) -> Result<(), SchedulerError> {
        let client = state.borrow().config.get_evm_client();

        let Some(initial_params) = state.borrow().config.get_evm_params() else {
            log::warn!("no evm parameters set, unable to update");
            return Err(SchedulerError::TaskExecutionFailed(
                "no evm parameters set".into(),
            ));
        };

        let address = {
            let signer = state.borrow().signer.get_transaction_signer();
            signer.get_address().await.into_scheduler_result()?
        };

        // Update the EvmParams
        log::trace!("updating evm params");
        let responses = query::batch_query(
            &client,
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

        state.borrow_mut().config.update_evm_params(|p| *p = params);
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

#[derive(Debug, Clone, Serialize, Deserialize, CandidType)]
pub struct BurntIcrc2Data {
    pub sender: Principal,
    pub amount: did::U256,
    pub src_token: Principal,
    pub recipient_address: did::H160,
    pub operation_id: u32,
    pub name: [u8; 32],
    pub symbol: [u8; 16],
    pub decimals: u8,
    pub fee_payer: Option<H160>,
}
