use std::cell::RefCell;
use std::rc::Rc;

use candid::Principal;
use did::H160;
use eth_signer::sign_strategy::TransactionSigner as _;
use ic_canister::{generate_idl, init, post_upgrade, query, update, Canister, Idl, PreUpdate};
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{CellStructure as _, StableUnboundedMap, VirtualMemory};
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::{Scheduler, TaskScheduler};
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, TaskOptions, TaskStatus};

use crate::constant::{
    EVM_INFO_INITIALIZATION_RETRIES, EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
    EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
};
use crate::interface::bridge_api::{BridgeError, Erc20MintStatus};
use crate::memory::{MEMORY_MANAGER, PENDING_TASKS_MEMORY_ID};
use crate::scheduler::Brc20Task;
use crate::state::{BftBridgeConfig, Brc20BridgeConfig, State};

#[derive(Canister, Clone, Debug)]
pub struct Brc20Bridge {
    #[id]
    id: Principal,
}

impl PreUpdate for Brc20Bridge {}

impl Brc20Bridge {
    #[init]
    pub fn init(&mut self, config: Brc20BridgeConfig) {
        get_state().borrow_mut().configure(config);

        {
            let scheduler = get_scheduler();
            let mut borrowed_scheduler = scheduler.borrow_mut();
            borrowed_scheduler.on_completion_callback(log_task_execution_error);
            borrowed_scheduler.append_task(Self::init_evm_info_task());
        }

        self.set_timers();
    }

    #[query]
    pub async fn get_deposit_address(&self) -> Result<String, BridgeError> {
        Ok(crate::ops::get_deposit_address(&get_state())
            .await?
            .to_string())
    }

    #[update]
    pub async fn brc20_to_erc20(
        &mut self,
        brc20_ticker: String,
        holder_btc_addr: String,
        dst_eth_addr: H160,
    ) -> Result<Erc20MintStatus, BridgeError> {
        crate::ops::brc20_to_erc20(&get_state(), dst_eth_addr, brc20_ticker, holder_btc_addr)
            .await
            .map_err(BridgeError::Erc20Mint)
    }

    /// Returns EVM address of the canister.
    #[update]
    pub async fn get_evm_address(&self) -> Option<H160> {
        let signer = get_state().borrow().signer().get().clone();
        match signer.get_address().await {
            Ok(address) => Some(address),
            Err(e) => {
                log::error!("failed to get EVM address of the canister: {e}");
                None
            }
        }
    }

    #[update]
    pub fn admin_configure_bft_bridge(&self, config: BftBridgeConfig) {
        get_state()
            .borrow()
            .check_admin(ic_exports::ic_kit::ic::caller());
        get_state().borrow_mut().configure_bft(config);
    }

    #[post_upgrade]
    pub fn post_upgrade(&mut self) {
        self.set_timers();
    }

    pub fn idl() -> Idl {
        generate_idl!()
    }

    fn set_timers(&mut self) {
        #[cfg(target_family = "wasm")]
        {
            use std::time::Duration;
            const METRICS_UPDATE_INTERVAL_SEC: u64 = 60 * 60;

            self.update_metrics_timer(Duration::from_secs(METRICS_UPDATE_INTERVAL_SEC));

            const GLOBAL_TIMER_INTERVAL: Duration = Duration::from_secs(1);
            ic_exports::ic_cdk_timers::set_timer_interval(GLOBAL_TIMER_INTERVAL, move || {
                get_scheduler()
                    .borrow_mut()
                    .append_task(Self::collect_evm_events_task());

                let task_execution_result = get_scheduler().borrow_mut().run();

                if let Err(err) = task_execution_result {
                    log::error!("task execution failed: {err}",);
                }
            });
        }
    }

    fn init_evm_info_task() -> ScheduledTask<Brc20Task> {
        let init_options = TaskOptions::default()
            .with_max_retries_policy(EVM_INFO_INITIALIZATION_RETRIES)
            .with_backoff_policy(BackoffPolicy::Exponential {
                secs: EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
                multiplier: EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
            });
        Brc20Task::InitEvmState.into_scheduled(init_options)
    }

    #[cfg(target_family = "wasm")]
    fn collect_evm_events_task() -> ScheduledTask<Brc20Task> {
        const EVM_EVENTS_COLLECTING_DELAY: u32 = 1;

        let options = TaskOptions::default()
            .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite)
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: EVM_EVENTS_COLLECTING_DELAY,
            });

        Brc20Task::CollectEvmEvents.into_scheduled(options)
    }
}

impl Metrics for Brc20Bridge {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

fn log_task_execution_error(task: InnerScheduledTask<Brc20Task>) {
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

pub type TasksStorage =
    StableUnboundedMap<u32, InnerScheduledTask<Brc20Task>, VirtualMemory<DefaultMemoryImpl>>;

pub type PersistentScheduler = Scheduler<Brc20Task, TasksStorage>;

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
