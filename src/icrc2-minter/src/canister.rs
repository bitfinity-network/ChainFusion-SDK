use std::cell::RefCell;
use std::rc::Rc;

use candid::Principal;
use did::build::BuildData;
use did::H160;
use eth_signer::sign_strategy::TransactionSigner;
use ic_canister::{
    generate_idl, init, post_upgrade, query, update, Canister, Idl, MethodType, PreUpdate,
};
use ic_exports::ic_cdk::api::management_canister::http_request::{HttpResponse, TransformArgs};
use ic_exports::ic_kit::ic;
use ic_log::writer::Logs;
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{StableBTreeMap, VirtualMemory};
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::{Scheduler, TaskScheduler};
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, TaskOptions, TaskStatus};
use log::*;
use minter_contract_utils::operation_store::{MinterOperationId, MinterOperationStore};
use minter_did::error::{Error, Result};
use minter_did::id256::Id256;
use minter_did::init::InitData;
use minter_did::order::SignedMintOrder;

use crate::build_data::canister_build_data;
use crate::constant::{
    OPERATIONS_LOG_MEMORY_ID, OPERATIONS_MAP_MEMORY_ID, OPERATIONS_MEMORY_ID,
    PENDING_TASKS_MEMORY_ID,
};
use crate::memory::MEMORY_MANAGER;
use crate::operation::OperationState;
use crate::state::{Settings, State};
use crate::tasks::BridgeTask;

mod inspect;

/// A canister to transfer funds between IC token canisters and EVM canister contracts.
#[derive(Canister, Clone)]
pub struct MinterCanister {
    #[id]
    id: Principal,
}

impl PreUpdate for MinterCanister {
    fn pre_update(&self, _method_name: &str, _method_type: MethodType) {}
}

impl MinterCanister {
    /// Initializes the timers
    pub fn set_timers(&mut self) {
        // This block of code only need to be run in the wasm environment
        #[cfg(target_family = "wasm")]
        {
            use std::time::Duration;

            self.update_metrics_timer(Duration::from_secs(60 * 60));

            const GLOBAL_TIMER_INTERVAL: Duration = Duration::from_secs(2);
            ic_exports::ic_cdk_timers::set_timer_interval(GLOBAL_TIMER_INTERVAL, move || {
                // Tasks to collect EVMs events
                let tasks = vec![Self::collect_evm_events_task()];

                get_scheduler().borrow_mut().append_tasks(tasks);

                let task_execution_result = get_scheduler().borrow_mut().run();

                if let Err(err) = task_execution_result {
                    log::error!("task execution failed: {err}",);
                }
            });
        }
    }

    fn init_evm_info_task() -> ScheduledTask<BridgeTask> {
        const EVM_INFO_INITIALIZATION_RETRIES: u32 = 5;
        const EVM_INFO_INITIALIZATION_RETRY_DELAY: u32 = 2;
        const EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER: u32 = 2;

        let init_options = TaskOptions::default()
            .with_max_retries_policy(EVM_INFO_INITIALIZATION_RETRIES)
            .with_backoff_policy(BackoffPolicy::Exponential {
                secs: EVM_INFO_INITIALIZATION_RETRY_DELAY,
                multiplier: EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
            });
        BridgeTask::InitEvmInfo.into_scheduled(init_options)
    }

    #[cfg(target_family = "wasm")]
    fn collect_evm_events_task() -> ScheduledTask<BridgeTask> {
        let options = TaskOptions::default();
        BridgeTask::CollectEvmEvents.into_scheduled(options)
    }

    /// Initialize the canister with given data.
    #[init]
    pub fn init(&mut self, init_data: InitData) {
        let state = get_state();
        let mut state = state.borrow_mut();

        if let Err(err) = state
            .logger_config_service
            .init(init_data.log_settings.clone().unwrap_or_default())
        {
            ic_exports::ic_cdk::println!("error configuring the logger. Err: {err:?}")
        }

        info!("starting minter canister");
        debug!("minter canister init data: [{init_data:?}]");

        check_anonymous_principal(init_data.owner).expect("anonymous principal not allowed");

        let settings = Settings {
            owner: init_data.owner,
            evm_principal: init_data.evm_principal,
            signing_strategy: init_data.signing_strategy,
        };

        state.reset(settings);

        {
            let scheduler = get_scheduler();
            let mut borrowed_scheduler = scheduler.borrow_mut();
            borrowed_scheduler.on_completion_callback(log_task_execution_error);
            borrowed_scheduler.append_task(Self::init_evm_info_task());
        }

        self.set_timers();
    }

    #[post_upgrade]
    pub fn post_upgrade(&mut self) {
        let state = get_state();
        let mut state = state.borrow_mut();

        if let Err(err) = state.logger_config_service.reload() {
            ic_exports::ic_cdk::println!("error configuring the logger. Err: {err:?}")
        }

        self.set_timers();
        debug!("upgrade completed");
    }

    /// set_logger_filter inspect_message check
    pub fn set_logger_filter_inspect_message_check(
        principal: Principal,
        state: &State,
    ) -> Result<()> {
        inspect_check_is_owner(principal, state)
    }

    /// Updates the runtime configuration of the logger with a new filter in the same form as the `RUST_LOG`
    /// environment variable.
    /// Example of valid filters:
    /// - info
    /// - debug,crate1::mod1=error,crate1::mod2,crate2=debug
    #[update]
    pub fn set_logger_filter(&mut self, filter: String) -> Result<()> {
        let state = get_state();
        let mut state = state.borrow_mut();

        MinterCanister::set_logger_filter_inspect_message_check(ic::caller(), &state)?;
        state.logger_config_service.set_logger_filter(&filter)?;

        debug!("updated logger filter to {filter}");

        Ok(())
    }

    /// ic_logs inspect_message check
    pub fn ic_logs_inspect_message_check(principal: Principal, state: &State) -> Result<()> {
        inspect_check_is_owner(principal, state)
    }

    /// Gets the logs
    /// - `count` is the number of logs to return
    #[update]
    pub fn ic_logs(&self, count: usize, offset: usize) -> Result<Logs> {
        MinterCanister::ic_logs_inspect_message_check(ic::caller(), &get_state().borrow())?;

        // Request execution
        Ok(ic_log::take_memory_records(count, offset))
    }

    /// Returns principal of canister owner.
    #[query]
    pub fn get_owner(&self) -> Principal {
        get_state().borrow().config.get_owner()
    }

    /// set_owner inspect_message check
    pub fn set_owner_inspect_message_check(
        principal: Principal,
        owner: Principal,
        state: &State,
    ) -> Result<()> {
        check_anonymous_principal(owner)?;
        inspect_check_is_owner(principal, state)
    }

    /// Sets a new principal for canister owner.
    ///
    /// This method should be called only by current owner,
    /// else `Error::NotAuthorised` will be returned.
    #[update]
    pub fn set_owner(&mut self, owner: Principal) -> Result<()> {
        let state = get_state();
        let mut state = state.borrow_mut();

        MinterCanister::set_owner_inspect_message_check(ic::caller(), owner, &state)?;
        state.config.set_owner(owner);

        info!("minter canister owner changed to {owner}");
        Ok(())
    }

    /// Returns principal of EVM canister with which the minter canister works.
    #[query]
    pub fn get_evm_principal(&self) -> Principal {
        get_state().borrow().config.get_evm_principal()
    }

    /// set_evm_principal inspect_message check
    pub fn set_evm_principal_inspect_message_check(
        principal: Principal,
        evm: Principal,
        state: &State,
    ) -> Result<()> {
        check_anonymous_principal(evm)?;
        inspect_check_is_owner(principal, state)
    }

    /// Sets principal of EVM canister with which the minter canister works.
    ///
    /// This method should be called only by current owner,
    /// else `Error::NotAuthorised` will be returned.
    #[update]
    pub fn set_evm_principal(&mut self, evm: Principal) -> Result<()> {
        let state = get_state();
        let mut state = state.borrow_mut();

        MinterCanister::set_evm_principal_inspect_message_check(ic::caller(), evm, &state)?;
        state.config.set_evm_principal(evm);

        info!("EVM principal changed to {evm}");
        Ok(())
    }

    /// Set BFT bridge contract address.
    #[update]
    pub async fn set_bft_bridge_contract(&mut self, address: H160) {
        get_state()
            .borrow_mut()
            .config
            .set_bft_bridge_contract(address);
    }

    /// Returns bridge contract address for EVM.
    /// If contract isn't initialized yet - returns None.
    #[query]
    pub fn get_bft_bridge_contract(&mut self) -> Option<H160> {
        get_state().borrow().config.get_bft_bridge_contract()
    }

    /// Returns `(nonce, mint_order)` pairs for the given sender id.
    /// Offset, if set, defines the starting index of the page,
    /// Count, if set, defines the number of elements in the page.
    #[query]
    pub fn list_mint_orders(
        &self,
        wallet_address: H160,
        src_token: Id256,
        offset: Option<usize>,
        count: Option<usize>,
    ) -> Vec<(u32, SignedMintOrder)> {
        Self::token_mint_orders(wallet_address, src_token, offset, count)
    }

    /// Returns `(nonce, mint_order)` pairs for the given sender id and operation_id.
    #[query]
    pub fn get_mint_order(
        &self,
        wallet_address: H160,
        src_token: Id256,
        operation_id: u32,
    ) -> Option<SignedMintOrder> {
        Self::token_mint_orders(wallet_address, src_token, None, None)
            .into_iter()
            .find(|(nonce, _)| *nonce == operation_id)
            .map(|(_, mint_order)| mint_order)
    }

    #[query]
    /// Returns the list of operations for the given wallet address.
    /// Offset, if set, defines the starting index of the page,
    /// Count, if set, defines the number of elements in the page.
    pub fn get_operations_list(
        &self,
        wallet_address: H160,
        offset: Option<usize>,
        count: Option<usize>,
    ) -> Vec<(MinterOperationId, OperationState)> {
        get_operations_store().get_for_address(&wallet_address, offset, count)
    }

    /// Returns evm_address of the minter canister.
    #[update]
    pub async fn get_minter_canister_evm_address(&mut self) -> Result<H160> {
        let signer = get_state().borrow().signer.get_transaction_signer();
        signer
            .get_address()
            .await
            .map_err(|e| Error::Internal(format!("failed to get minter canister address: {e}")))
    }

    /// Returns the build data of the canister
    #[query]
    pub fn get_canister_build_data(&self) -> BuildData {
        canister_build_data()
    }

    /// Adds the provided principal to the whitelist.
    #[update]
    pub fn add_to_whitelist(&mut self, icrc2_principal: Principal) -> Result<()> {
        let state = get_state();

        Self::access_control_inspect_message_check(ic::caller(), icrc2_principal, &state.borrow())?;

        let mut state = state.borrow_mut();

        state.access_list.add(icrc2_principal)?;

        Ok(())
    }

    /// Remove a icrc2 principal token from the access list
    #[update]
    pub fn remove_from_whitelist(&mut self, icrc2_principal: Principal) -> Result<()> {
        let state = get_state();

        Self::access_control_inspect_message_check(ic::caller(), icrc2_principal, &state.borrow())?;

        let mut state = state.borrow_mut();

        state.access_list.remove(&icrc2_principal);

        Ok(())
    }

    /// Returns the list of all principals in the whitelist.
    #[query]
    fn get_whitelist(&self) -> Vec<Principal> {
        get_state().borrow().access_list.get_all_principals()
    }

    fn access_control_inspect_message_check(
        owner: Principal,
        icrc2_principal: Principal,
        state: &State,
    ) -> Result<()> {
        inspect_check_is_owner(owner, state)?;
        check_anonymous_principal(icrc2_principal)?;

        Ok(())
    }

    /// Requirements for Http outcalls, used to ignore small differences in the data obtained
    /// by different nodes of the IC subnet to reach a consensus, more info:
    /// https://internetcomputer.org/docs/current/developer-docs/integrations/http_requests/http_requests-how-it-works#transformation-function
    #[query]
    fn transform(&self, raw: TransformArgs) -> HttpResponse {
        HttpResponse {
            status: raw.response.status,
            headers: raw.response.headers,
            body: raw.response.body,
        }
    }

    /// Returns candid IDL.
    /// This should be the last fn to see previous endpoints in macro.
    pub fn idl() -> Idl {
        generate_idl!()
    }

    /// Get mint orders for the given wallet address and token;
    /// if `offset` and `count` are provided, returns a page of mint orders.
    fn token_mint_orders(
        wallet_address: H160,
        src_token: Id256,
        offset: Option<usize>,
        count: Option<usize>,
    ) -> Vec<(u32, SignedMintOrder)> {
        get_operations_store()
            .get_for_address(&wallet_address, None, None)
            .into_iter()
            .filter_map(|(operation_id, status)| {
                status
                    .get_signed_mint_order(Some(src_token))
                    .map(|mint_order| (operation_id.nonce(), *mint_order))
            })
            .skip(offset.unwrap_or_default())
            .take(count.unwrap_or(usize::MAX))
            .collect()
    }
}

impl Metrics for MinterCanister {
    fn metrics(&self) -> Rc<RefCell<ic_metrics::MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

/// inspect function to check whether provided principal is owner
fn inspect_check_is_owner(principal: Principal, state: &State) -> Result<()> {
    let owner = state.config.get_owner();

    if owner != principal {
        return Err(Error::NotAuthorized);
    }

    Ok(())
}

/// inspect function to check whether the provided principal is anonymous
fn check_anonymous_principal(principal: Principal) -> Result<()> {
    if principal == Principal::anonymous() {
        return Err(Error::AnonymousPrincipal);
    }

    Ok(())
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
        status_change => {
            log::trace!("task #{} status changed: {status_change:?}", task.id())
        }
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

pub fn get_scheduler() -> Rc<RefCell<PersistentScheduler>> {
    SCHEDULER.with(|scheduler| scheduler.clone())
}

pub fn get_state() -> Rc<RefCell<State>> {
    STATE.with(|state| state.clone())
}

pub fn get_operations_store(
) -> MinterOperationStore<VirtualMemory<DefaultMemoryImpl>, OperationState> {
    MEMORY_MANAGER.with(|mm| {
        MinterOperationStore::with_memory(
            mm.get(OPERATIONS_MEMORY_ID),
            mm.get(OPERATIONS_LOG_MEMORY_ID),
            mm.get(OPERATIONS_MAP_MEMORY_ID),
            None,
        )
    })
}

#[cfg(test)]
mod test {
    use candid::Principal;
    use did::U256;
    use eth_signer::sign_strategy::SigningStrategy;
    use ic_canister::{canister_call, Canister};
    use ic_exports::ic_kit::{inject, MockContext};
    use minter_did::error::Error;

    use super::*;
    use crate::operation::DepositOperationState;
    use crate::MinterCanister;

    fn owner() -> Principal {
        Principal::from_slice(&[1; 20])
    }

    fn bob() -> Principal {
        Principal::from_slice(&[2; 20])
    }

    async fn init_canister() -> MinterCanister {
        MockContext::new().inject();

        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");
        let mut canister = MinterCanister::from_principal(mock_canister_id);

        let init_data = InitData {
            owner: owner(),
            evm_principal: Principal::anonymous(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();
        canister
    }

    #[tokio::test]
    #[should_panic = "anonymous principal not allowed"]
    async fn disallow_anonymous_owner_in_init() {
        MockContext::new().inject();
        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");
        let mut canister = MinterCanister::from_principal(mock_canister_id);

        let init_data = InitData {
            owner: Principal::anonymous(),
            evm_principal: Principal::anonymous(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();
    }

    #[tokio::test]
    async fn correct_initialization() {
        let canister = init_canister().await;

        let stored_owner = canister_call!(canister.get_owner(), Principal)
            .await
            .unwrap();
        assert_eq!(stored_owner, owner());

        let stored_evm = canister_call!(canister.get_evm_principal(), Principal)
            .await
            .unwrap();
        assert_eq!(stored_evm, Principal::anonymous());
    }

    #[tokio::test]
    async fn owner_access_control() {
        let mut canister = init_canister().await;

        // try to call with not owner id
        let set_error = canister_call!(canister.set_owner(bob()), Result<()>)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(set_error, Error::NotAuthorized);

        // now we will try to call it with owner id
        inject::get_context().update_id(owner());
        canister_call!(canister.set_owner(bob()), Result<()>)
            .await
            .unwrap()
            .unwrap();

        // check if state updated
        let stored_owner = canister_call!(canister.get_owner(), Principal)
            .await
            .unwrap();
        assert_eq!(stored_owner, bob());
    }

    #[tokio::test]
    async fn evm_principal_access_control() {
        let mut canister = init_canister().await;

        // try to call with not owner id
        let set_error = canister_call!(canister.set_evm_principal(bob()), Result<()>)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(set_error, Error::NotAuthorized);

        // now we will try to call it with owner id
        inject::get_context().update_id(owner());
        canister_call!(canister.set_evm_principal(bob()), Result<()>)
            .await
            .unwrap()
            .unwrap();

        // check if state updated
        let stored_owner = canister_call!(canister.get_evm_principal(), Principal)
            .await
            .unwrap();
        assert_eq!(stored_owner, bob());
    }

    #[tokio::test]
    async fn set_anonymous_principal_as_owner() {
        let mut canister = init_canister().await;

        inject::get_context().update_id(owner());

        let err = canister_call!(canister.set_owner(Principal::anonymous()), Result<()>)
            .await
            .unwrap()
            .unwrap_err();

        assert_eq!(err, Error::AnonymousPrincipal);
    }

    // This test work fine if executed alone but could fail if executed with all other tests
    // due to the global nature of the global logger in Rust.
    // In fact, if the Rust log is already set, a second attempt to set it causes a panic
    #[ignore]
    #[tokio::test]
    async fn test_set_logger_filter() {
        MockContext::new().inject();
        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");
        let mut canister = MinterCanister::from_principal(mock_canister_id);

        let init_data = InitData {
            owner: Principal::anonymous(),
            evm_principal: Principal::anonymous(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();

        {
            let info_message = format!("message-{}", rand::random::<u64>());
            let error_message = format!("message-{}", rand::random::<u64>());

            log::info!("{info_message}");
            log::error!("{error_message}");

            // Only the error message should be present
            let log_records = ic_log::take_memory_records(128, 0);
            assert!(!log_records
                .logs
                .iter()
                .any(|log| log.log.contains(&info_message)));
            assert!(log_records
                .logs
                .iter()
                .any(|log| log.log.contains(&error_message)));
        }
        // Set new logger filter
        let new_filter = "info";
        let res = canister_call!(
            canister.set_logger_filter(new_filter.to_string()),
            Result<()>
        )
        .await
        .unwrap();
        assert!(res.is_ok());

        {
            let info_message = format!("message-{}", rand::random::<u64>());
            let error_message = format!("message-{}", rand::random::<u64>());

            log::info!("{info_message}");
            log::error!("{error_message}");

            // All log messages should be present
            let log_records = ic_log::take_memory_records(128, 0);
            assert!(log_records
                .logs
                .iter()
                .any(|log| log.log.contains(&info_message)));
            assert!(log_records
                .logs
                .iter()
                .any(|log| log.log.contains(&error_message)));
        }
    }

    #[tokio::test]
    async fn test_ic_logs_is_access_controlled() {
        MockContext::new().inject();
        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");
        let mut canister = MinterCanister::from_principal(mock_canister_id);

        let init_data = InitData {
            owner: Principal::management_canister(),
            evm_principal: Principal::management_canister(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();

        inject::get_context().update_id(Principal::management_canister());

        let logs = canister_call!(canister.ic_logs(10, 0), Result<Logs>)
            .await
            .unwrap();
        assert!(logs.is_ok());

        let init_data = InitData {
            owner: Principal::management_canister(),
            evm_principal: Principal::management_canister(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();

        inject::get_context().update_id(Principal::anonymous());

        let logs = canister_call!(canister.ic_logs(10, 0), Result<Logs>)
            .await
            .unwrap();
        assert!(logs.is_err());
        assert_eq!(logs.unwrap_err(), Error::NotAuthorized);
    }

    #[tokio::test]
    async fn test_get_minter_canister_evm_address() {
        MockContext::new().inject();
        const MOCK_PRINCIPAL: &str = "mfufu-x6j4c-gomzb-geilq";
        let mock_canister_id = Principal::from_text(MOCK_PRINCIPAL).expect("valid principal");
        let mut canister = MinterCanister::from_principal(mock_canister_id);

        let init_data = InitData {
            owner: Principal::management_canister(),
            evm_principal: Principal::management_canister(),
            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
            log_settings: None,
        };
        canister_call!(canister.init(init_data), ()).await.unwrap();
        inject::get_context().update_id(Principal::management_canister());

        let evm_address = canister_call!(canister.get_minter_canister_evm_address(), Result<H160>)
            .await
            .unwrap();

        assert!(evm_address.is_ok());
    }

    #[tokio::test]
    async fn test_access_list() {
        let mut canister = init_canister().await;

        let icrc2_principal = Principal::from_text("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap();

        // Add to whitelist
        inject::get_context().update_id(owner());
        canister_call!(canister.add_to_whitelist(icrc2_principal), Result<()>)
            .await
            .unwrap()
            .unwrap();

        // Check if the principal is in the whitelist
        let whitelist = canister_call!(canister.get_whitelist(), Vec<Principal>)
            .await
            .unwrap();
        assert_eq!(whitelist, vec![icrc2_principal]);

        // Remove from whitelist
        canister_call!(canister.remove_from_whitelist(icrc2_principal), Result<()>)
            .await
            .unwrap()
            .unwrap();

        // Check if the principal is removed from the whitelist
        let whitelist = canister_call!(canister.get_whitelist(), Vec<Principal>)
            .await
            .unwrap();

        assert!(whitelist.is_empty());
    }

    #[tokio::test]
    async fn test_should_paginate_token_mint_orders() {
        fn eth_address(seed: u8) -> H160 {
            H160::from([seed; H160::BYTE_SIZE])
        }

        let token_id = eth_address(0);
        let token_id_id256 = Id256::from_evm_address(&token_id, 5);

        let op_state = OperationState::Deposit(DepositOperationState::MintOrderSigned {
            token_id: token_id_id256,
            amount: U256::one(),
            signed_mint_order: Box::new(SignedMintOrder([0; 334])),
        });

        let token_id_other = eth_address(1);
        let token_id_other_id256 = Id256::from_evm_address(&token_id_other, 5);

        let op_state_other = OperationState::Deposit(DepositOperationState::MintOrderSigned {
            token_id: token_id_other_id256,
            amount: U256::one(),
            signed_mint_order: Box::new(SignedMintOrder([0; 334])),
        });

        const COUNT: usize = 42;
        const COUNT_OTHER: usize = 10;

        let canister = init_canister().await;

        inject::get_context().update_id(owner());
        let mut op_store = get_operations_store();

        let owner = eth_address(2);
        let owner_other = eth_address(3);

        for _ in 0..COUNT {
            op_store.new_operation(owner.clone(), op_state.clone());
        }

        for _ in 0..COUNT_OTHER {
            op_store.new_operation(owner_other.clone(), op_state_other.clone());
        }

        // get orders for the first token
        let orders = canister_call!(
            canister.list_mint_orders(owner.clone(), token_id_id256, None, Some(COUNT)),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();

        assert_eq!(orders.len(), COUNT);

        // get with offset
        let orders = canister_call!(
            canister.list_mint_orders(owner.clone(), token_id_id256, Some(10), Some(20)),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert_eq!(orders.len(), 20);

        // get with offset to the end
        let orders = canister_call!(
            canister.list_mint_orders(owner.clone(), token_id_id256, Some(COUNT - 5), Some(100)),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert_eq!(orders.len(), 5);

        // get orders with no limit
        let orders = canister_call!(
            canister.list_mint_orders(owner.clone(), token_id_id256, None, None),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert_eq!(orders.len(), COUNT);

        // get orders with offset but no limit
        let orders = canister_call!(
            canister.list_mint_orders(owner.clone(), token_id_id256, Some(10), None),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert_eq!(orders.len(), COUNT - 10);

        // get orders for the second token but `owner`
        let orders = canister_call!(
            canister.list_mint_orders(owner, token_id_other_id256, None, None),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert!(orders.is_empty());

        // get orders for the second token
        let orders = canister_call!(
            canister.list_mint_orders(owner_other.clone(), token_id_other_id256, None, None),
            Vec<(u32, SignedMintOrder)>
        )
        .await
        .unwrap();
        assert_eq!(orders.len(), COUNT_OTHER);
    }
}
