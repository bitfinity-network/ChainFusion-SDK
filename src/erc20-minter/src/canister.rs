use std::cell::RefCell;
use std::rc::Rc;

use candid::Principal;
use did::{H160, H256, U256};
use eth_signer::sign_strategy::TransactionSigner;
use ic_canister::{generate_idl, init, post_upgrade, query, update, Canister, Idl, PreUpdate};
use ic_exports::ic_kit::ic;
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{CellStructure, StableBTreeMap, VirtualMemory};
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::{Scheduler, TaskScheduler};
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, TaskOptions, TaskStatus};
use minter_contract_utils::evm_bridge::{BftBridgeInitArgs, BridgeSide};
use minter_did::error::Result;
use minter_did::id256::Id256;
use minter_did::order::SignedMintOrder;

use crate::memory::{MEMORY_MANAGER, PENDING_TASKS_MEMORY_ID};
use crate::state::{Settings, State};
use crate::tasks::BridgeTask;

const EVM_INFO_INITIALIZATION_RETRIES: u32 = 5;
const EVM_INFO_INITIALIZATION_RETRY_DELAY: u32 = 2;
const EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER: u32 = 2;

#[derive(Canister, Clone, Debug)]
pub struct EvmMinter {
    #[id]
    id: Principal,
}

impl PreUpdate for EvmMinter {}

impl EvmMinter {
    fn set_timers(&mut self) {
        // Set the metrics updating interval
        #[cfg(target_family = "wasm")]
        {
            use std::time::Duration;

            self.update_metrics_timer(std::time::Duration::from_secs(60 * 60));

            const GLOBAL_TIMER_INTERVAL: Duration = Duration::from_secs(1);
            ic_exports::ic_cdk_timers::set_timer_interval(GLOBAL_TIMER_INTERVAL, move || {
                // Tasks to collect EVMs events
                let tasks = vec![
                    Self::collect_evm_events_task(BridgeSide::Base),
                    Self::collect_evm_events_task(BridgeSide::Wrapped),
                ];

                get_scheduler().borrow_mut().append_tasks(tasks);

                let task_execution_result = get_scheduler().borrow_mut().run();

                if let Err(err) = task_execution_result {
                    log::error!("task execution failed: {err}",);
                }
            });
        }
    }

    #[init]
    pub fn init(&mut self, settings: Settings) {
        let admin = ic::caller();

        Self::check_anonymous_principal(admin).expect("admin principal is anonymous");

        let state = get_state();
        state.borrow_mut().init(admin, settings);

        log::info!("starting erc20-minter canister");

        let tasks = vec![
            // Tasks to init EVMs state
            Self::init_evm_info_task(BridgeSide::Base),
            Self::init_evm_info_task(BridgeSide::Wrapped),
        ];

        {
            let scheduler = get_scheduler();
            let mut borrowed_scheduler = scheduler.borrow_mut();
            borrowed_scheduler.on_completion_callback(log_task_execution_error);
            borrowed_scheduler.append_tasks(tasks);
        }

        self.set_timers();

        log::info!("erc20-minter canister initialized");
    }

    fn init_evm_info_task(bridge_side: BridgeSide) -> ScheduledTask<BridgeTask> {
        let init_options = TaskOptions::default()
            .with_max_retries_policy(EVM_INFO_INITIALIZATION_RETRIES)
            .with_backoff_policy(BackoffPolicy::Exponential {
                secs: EVM_INFO_INITIALIZATION_RETRY_DELAY,
                multiplier: EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
            });
        BridgeTask::InitEvmState(bridge_side).into_scheduled(init_options)
    }

    #[cfg(target_family = "wasm")]
    fn collect_evm_events_task(bridge_side: BridgeSide) -> ScheduledTask<BridgeTask> {
        const EVM_EVENTS_COLLECTING_DELAY: u32 = 1;

        let options = TaskOptions::default()
            .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite)
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: EVM_EVENTS_COLLECTING_DELAY,
            });

        BridgeTask::CollectEvmEvents(bridge_side).into_scheduled(options)
    }

    #[post_upgrade]
    pub fn post_upgrade(&mut self) {
        self.set_timers();
    }

    /// Returns `(operaion_id, signed_mint_order)` pairs for the given sender id.
    #[query]
    pub async fn list_mint_orders(
        &self,
        sender: Id256,
        src_token: Id256,
    ) -> Vec<(u32, SignedMintOrder)> {
        get_state().borrow().mint_orders.get_all(sender, src_token)
    }

    /// Returns the `signed_mint_order` if present.
    #[query]
    pub async fn get_mint_order(
        &self,
        sender: Id256,
        src_token: Id256,
        operation_id: u32,
    ) -> Option<SignedMintOrder> {
        get_state()
            .borrow()
            .mint_orders
            .get(sender, src_token, operation_id)
    }

    /// Returns EVM address of the canister.
    #[update]
    pub async fn get_evm_address(&self) -> Option<H160> {
        let signer = get_state().borrow().signer.get().clone();
        match signer.get_address().await {
            Ok(address) => Some(address),
            Err(e) => {
                log::error!("failed to get EVM address: {e}");
                None
            }
        }
    }

    /// Starts the BFT bridge contract deployment.
    #[update]
    pub async fn init_bft_bridge_contract(
        &mut self,
        side: BridgeSide,
        fee_charge_address: H160,
    ) -> Result<H256> {
        let state = get_state();
        let signer = state.borrow().signer.get().clone();

        let evm_info = state.borrow().config.get_evm_info(side);
        let evm_link = evm_info.link;
        let evm_params = evm_info
            .params
            .ok_or_else(|| "EVM params not initialized".to_string())?;
        let minter_address = signer.get_address().await.map_err(|e| e.to_string())?;

        let mut status = state.borrow().config.get_bft_bridge_status(side);

        log::trace!("Starting BftBridge contract initialization with current status: {status:?}");

        let hash = status
            .initialize(BftBridgeInitArgs::new(
                evm_link,
                evm_params.chain_id as _,
                Box::new(signer),
                minter_address,
                fee_charge_address,
                side == BridgeSide::Wrapped,
            ))
            .await
            .map_err(|e| e.to_string())?;

        log::trace!("BftBridge contract initialization started with status: {status:?}");

        state
            .borrow_mut()
            .config
            .set_bft_bridge_status(side, status);

        let options = TaskOptions::default()
            .with_max_retries_policy(10)
            .with_fixed_backoff_policy(4);
        get_scheduler()
            .borrow_mut()
            .append_task(ScheduledTask::with_options(
                BridgeTask::RefreshBftBridgeCreationStatus(side),
                options,
            ));

        Ok(hash)
    }

    fn check_anonymous_principal(principal: Principal) -> minter_did::error::Result<()> {
        if principal == Principal::anonymous() {
            return Err(minter_did::error::Error::AnonymousPrincipal);
        }

        Ok(())
    }

    pub fn idl() -> Idl {
        generate_idl!()
    }
}

impl Metrics for EvmMinter {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

type TasksStorage =
    StableBTreeMap<u32, InnerScheduledTask<BridgeTask>, VirtualMemory<DefaultMemoryImpl>>;
type PersistentScheduler = Scheduler<BridgeTask, TasksStorage>;

fn log_task_execution_error(task: InnerScheduledTask<BridgeTask>) {
    match task.status() {
        TaskStatus::Failed {
            timestamp_secs,
            error,
        } => {
            log::error!(
                "task #{} execution failed: {error} at {timestamp_secs}",
                task.id()
            )
        }
        TaskStatus::TimeoutOrPanic { timestamp_secs } => {
            log::error!("task #{} panicked at {timestamp_secs}", task.id())
        }
        _ => (),
    };
}

thread_local! {
    pub static STATE: Rc<RefCell<State>> = Rc::default();

    pub static SCHEDULER: Rc<RefCell<PersistentScheduler>> = Rc::new(RefCell::new({
        let pending_tasks =
            TasksStorage::new(MEMORY_MANAGER.with(|mm| mm.get(PENDING_TASKS_MEMORY_ID)));
            PersistentScheduler::new(pending_tasks)
    }));
}

pub fn get_state() -> Rc<RefCell<State>> {
    STATE.with(|state| state.clone())
}

pub fn get_scheduler() -> Rc<RefCell<PersistentScheduler>> {
    SCHEDULER.with(|scheduler| scheduler.clone())
}

#[cfg(test)]
mod test {
    use candid::Principal;
    use eth_signer::sign_strategy::SigningStrategy;
    use ic_canister::{canister_call, Canister};
    use ic_exports::ic_kit::inject::{self};
    use ic_exports::ic_kit::MockContext;
    use minter_contract_utils::evm_link::EvmLink;

    use super::*;
    use crate::EvmMinter;

    #[tokio::test]
    #[should_panic = "admin principal is anonymous"]
    async fn disallow_anonymous_owner_in_init() {
        MockContext::new().inject();
        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");

        inject::get_context().update_id(Principal::anonymous());

        let mut canister = EvmMinter::from_principal(mock_canister_id);

        let init_data = Settings {
            base_evm_link: EvmLink::Http("".to_string()),
            wrapped_evm_link: EvmLink::Http("".to_string()),
            signing_strategy: SigningStrategy::Local {
                private_key: [0; 32],
            },
            log_settings: None,
        };

        canister_call!(canister.init(init_data), ()).await.unwrap();
    }
}
