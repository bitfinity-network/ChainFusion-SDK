use std::cell::RefCell;
use std::rc::Rc;

use candid::Principal;
use did::H160;
use eth_signer::sign_strategy::TransactionSigner as _;
use ic_canister::{generate_idl, init, post_upgrade, query, update, Canister, Idl, PreUpdate};
use ic_exports::ic_cdk::api::management_canister::ecdsa::{
    ecdsa_public_key, EcdsaPublicKeyArgument,
};
use ic_exports::ic_kit::ic;
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{CellStructure as _, StableUnboundedMap, VirtualMemory};
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::{Scheduler, TaskScheduler};
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, TaskOptions, TaskStatus};
use inscriber::interface::{
    InscribeResult, InscribeTransactions, InscriptionFees, Multisig, Protocol,
};
use inscriber::ops as Inscriber;

use crate::build_data::BuildData;
use crate::constant::{
    EVM_INFO_INITIALIZATION_RETRIES, EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
    EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
};
use crate::interface;
use crate::interface::bridge_api::{BridgeError, NftMintStatus};
use crate::interface::store::StorableInscriptionId;
use crate::memory::{MEMORY_MANAGER, PENDING_TASKS_MEMORY_ID};
use crate::scheduler::NftTask;
use crate::state::{BtcNftBridgeConfig, NftBridgeConfig, State};

#[derive(Canister, Clone, Debug)]
pub struct NftBridge {
    #[id]
    id: Principal,
}

impl PreUpdate for NftBridge {}

impl NftBridge {
    #[init]
    pub fn init(&mut self, config: BtcNftBridgeConfig) {
        get_state().borrow_mut().configure(config);

        {
            let scheduler = get_scheduler();
            let mut borrowed_scheduler = scheduler.borrow_mut();
            borrowed_scheduler.on_completion_callback(log_task_execution_error);
            borrowed_scheduler.append_task(Self::init_evm_info_task());
        }

        self.set_timers();
    }

    #[update]
    pub async fn get_deposit_address(&mut self, eth_address: H160) -> String {
        let network = { get_state().borrow().ic_btc_network() };
        interface::get_deposit_address(&get_state(), &eth_address, network).await
    }

    /// Returns the balance of the given bitcoin address.
    #[update]
    pub async fn get_balance(&mut self, address: String) -> u64 {
        use inscriber::interface::bitcoin_api;

        let network = get_state().borrow().ic_btc_network();
        bitcoin_api::get_balance(network, address).await
    }

    #[update]
    pub async fn get_inscription_fees(
        &self,
        inscription_type: Protocol,
        inscription: String,
        multisig_config: Option<Multisig>,
    ) -> InscribeResult<InscriptionFees> {
        let (network, ecdsa_signer) = {
            (
                get_state().borrow().ic_btc_network(),
                get_state().borrow().ecdsa_signer(),
            )
        };
        Inscriber::get_inscription_fees(
            inscription_type,
            inscription,
            multisig_config,
            network,
            ecdsa_signer,
        )
        .await
    }

    /// Inscribes and sends the inscribed sat from this canister to the given address.
    /// Returns the commit and reveal transaction IDs.
    #[update]
    pub async fn inscribe(
        &mut self,
        inscription_type: Protocol,
        inscription: String,
        leftovers_address: String,
        dst_address: String,
        multisig_config: Option<Multisig>,
    ) -> InscribeResult<InscribeTransactions> {
        let (network, ecdsa_signer) = {
            (
                get_state().borrow().ic_btc_network(),
                get_state().borrow().ecdsa_signer(),
            )
        };

        Inscriber::inscribe(
            inscription_type,
            inscription,
            leftovers_address,
            dst_address,
            multisig_config,
            ecdsa_signer,
            network,
        )
        .await
    }

    #[update]
    pub async fn nft_to_erc721(
        &mut self,
        nft_id: StorableInscriptionId,
        holder_btc_addr: String,
        dst_eth_addr: H160,
    ) -> Result<NftMintStatus, BridgeError> {
        crate::ops::nft_to_erc721(&get_state(), dst_eth_addr, nft_id.into(), holder_btc_addr)
            .await
            .map_err(BridgeError::Erc721Mint)
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
    pub fn admin_configure_nft_bridge(&self, config: NftBridgeConfig) {
        get_state()
            .borrow()
            .check_admin(ic_exports::ic_kit::ic::caller());
        get_state().borrow_mut().configure_bft(config);
    }

    #[update]
    pub async fn admin_configure_ecdsa(&self) {
        get_state().borrow().check_admin(ic::caller());
        let key_id = get_state().borrow().ecdsa_key_id();

        let master_key = ecdsa_public_key(EcdsaPublicKeyArgument {
            canister_id: None,
            derivation_path: vec![],
            key_id,
        })
        .await
        .expect("failed to get master key");

        get_state().borrow_mut().configure_ecdsa(master_key.0);
    }

    #[post_upgrade]
    pub fn post_upgrade(&mut self) {
        self.set_timers();
    }

    #[query]
    pub fn get_canister_build_data(&self) -> BuildData {
        crate::build_data::canister_build_data()
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

    fn init_evm_info_task() -> ScheduledTask<NftTask> {
        let init_options = TaskOptions::default()
            .with_max_retries_policy(EVM_INFO_INITIALIZATION_RETRIES)
            .with_backoff_policy(BackoffPolicy::Exponential {
                secs: EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
                multiplier: EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
            });
        NftTask::InitEvmState.into_scheduled(init_options)
    }

    #[cfg(target_family = "wasm")]
    fn collect_evm_events_task() -> ScheduledTask<NftTask> {
        const EVM_EVENTS_COLLECTING_DELAY: u32 = 1;

        let options = TaskOptions::default()
            .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite)
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: EVM_EVENTS_COLLECTING_DELAY,
            });

        NftTask::CollectEvmEvents.into_scheduled(options)
    }
}

impl Metrics for NftBridge {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

fn log_task_execution_error(task: InnerScheduledTask<NftTask>) {
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
    StableUnboundedMap<u32, InnerScheduledTask<NftTask>, VirtualMemory<DefaultMemoryImpl>>;

pub type PersistentScheduler = Scheduler<NftTask, TasksStorage>;

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
