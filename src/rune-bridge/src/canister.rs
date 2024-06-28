use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use bitcoin::bip32::DerivationPath;
use bitcoin::consensus::Encodable;
use bitcoin::hashes::sha256d::Hash;
use bitcoin::{Address, Amount, OutPoint, TxOut, Txid};
use candid::Principal;
use did::H160;
use eth_signer::sign_strategy::TransactionSigner;
use ic_canister::{generate_idl, init, post_upgrade, query, update, Canister, Idl, PreUpdate};
use ic_exports::ic_cdk::api::management_canister::ecdsa::{
    ecdsa_public_key, EcdsaPublicKeyArgument,
};
use ic_exports::ic_kit::ic;
use ic_exports::ledger::Subaccount;
use ic_metrics::{Metrics, MetricsStorage};
use ic_stable_structures::CellStructure;
use ic_task_scheduler::retry::BackoffPolicy;
use ic_task_scheduler::scheduler::TaskScheduler;
use ic_task_scheduler::task::{InnerScheduledTask, ScheduledTask, TaskOptions, TaskStatus};
use minter_contract_utils::operation_store::{MinterOperationId, MinterOperationStore};
use ord_rs::wallet::{ScriptType, TxInputInfo};
use ord_rs::OrdTransactionBuilder;

use crate::core::deposit::RuneDeposit;
use crate::core::index_provider::{OrdIndexProvider, RuneIndexProvider};
use crate::core::utxo_provider::{IcUtxoProvider, UtxoProvider};
use crate::interface::{CreateEdictTxArgs, GetAddressError, WithdrawError};
use crate::memory::{
    MEMORY_MANAGER, OPERATIONS_LOG_MEMORY_ID, OPERATIONS_MAP_MEMORY_ID, OPERATIONS_MEMORY_ID,
    PENDING_TASKS_MEMORY_ID,
};
use crate::operation::{OperationState, RuneOperationStore};
use crate::rune_info::RuneInfo;
use crate::scheduler::{PersistentScheduler, RuneBridgeTask, TasksStorage};
use crate::state::{BftBridgeConfig, RuneBridgeConfig, State};
use crate::{
    EVM_INFO_INITIALIZATION_RETRIES, EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
    EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
};

#[derive(Canister, Clone, Debug)]
pub struct RuneBridge {
    #[id]
    id: Principal,
}

impl PreUpdate for RuneBridge {}

impl RuneBridge {
    fn set_timers(&mut self) {
        #[cfg(target_family = "wasm")]
        {
            use std::time::Duration;
            const METRICS_UPDATE_INTERVAL_SEC: u64 = 60 * 60;

            self.update_metrics_timer(std::time::Duration::from_secs(METRICS_UPDATE_INTERVAL_SEC));

            const GLOBAL_TIMER_INTERVAL: Duration = Duration::from_secs(1);
            const USED_UTXOS_REMOVE_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24); // once a day

            ic_exports::ic_cdk_timers::set_timer_interval(GLOBAL_TIMER_INTERVAL, move || {
                get_scheduler()
                    .borrow_mut()
                    .append_task(Self::collect_evm_events_task());

                let task_execution_result = get_scheduler().borrow_mut().run();

                if let Err(err) = task_execution_result {
                    log::error!("task execution failed: {err}",);
                }
            });

            ic_exports::ic_cdk_timers::set_timer_interval(USED_UTXOS_REMOVE_INTERVAL, || {
                ic_exports::ic_cdk::spawn(
                    crate::task::RemoveUsedUtxosTask::from(get_state()).run(),
                );
            });
        }
    }

    #[init]
    pub fn init(&mut self, config: RuneBridgeConfig) {
        get_state().borrow_mut().configure(config);

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
        self.set_timers();
    }


    /// Returns the bitcoin address that a user has to use to deposit runes to be received on the given Ethereum address.
    #[query]
    pub fn get_deposit_address(&self, eth_address: H160) -> Result<String, GetAddressError> {
        crate::key::get_transit_address(&get_state(), &eth_address).map(|v| v.to_string())
    }

    #[query]
    pub fn get_operations_list(
        &self,
        wallet_address: H160,
    ) -> Vec<(MinterOperationId, OperationState)> {
        get_operations_store().get_for_address(&wallet_address)
    }

    fn init_evm_info_task() -> ScheduledTask<RuneBridgeTask> {
        let init_options = TaskOptions::default()
            .with_max_retries_policy(EVM_INFO_INITIALIZATION_RETRIES)
            .with_backoff_policy(BackoffPolicy::Exponential {
                secs: EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC,
                multiplier: EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER,
            });
        RuneBridgeTask::InitEvmState.into_scheduled(init_options)
    }

    /// Returns EVM address of the canister.
    #[update]
    pub async fn get_evm_address(&self) -> Option<H160> {
        let signer = get_state().borrow().signer().get().clone();
        match signer.get_address().await {
            Ok(address) => Some(address),
            Err(e) => {
                log::error!("failed to get EVM address: {e}");
                None
            }
        }
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

    #[update]
    pub fn admin_configure_bft_bridge(&self, config: BftBridgeConfig) {
        get_state().borrow().check_admin(ic::caller());
        get_state().borrow_mut().configure_bft(config);
    }

    #[cfg(target_family = "wasm")]
    fn collect_evm_events_task() -> ScheduledTask<RuneBridgeTask> {
        const EVM_EVENTS_COLLECTING_DELAY: u32 = 1;

        let options = TaskOptions::default()
            .with_retry_policy(ic_task_scheduler::retry::RetryPolicy::Infinite)
            .with_backoff_policy(BackoffPolicy::Fixed {
                secs: EVM_EVENTS_COLLECTING_DELAY,
            });

        RuneBridgeTask::CollectEvmEvents.into_scheduled(options)
    }

    #[update]
    pub async fn get_rune_balances(&self, btc_address: String) -> Vec<(RuneInfo, u128)> {
        let address = Address::from_str(&btc_address)
            .expect("invalid address")
            .assume_checked();

        let deposit = RuneDeposit::get();
        let utxos = deposit
            .get_deposit_utxos(&address)
            .await
            .expect("failed to get utxos");
        let (rune_info_amounts, _) = deposit
            .get_mint_amounts(&utxos.utxos, &None)
            .await
            .expect("failed to get rune amounts");

        rune_info_amounts
    }

    /// This was implemented for testing purposes.
    /// It is not used in the production code.
    /// To be removed. See: EPROD-935
    #[update]
    pub async fn create_edict_tx(&self, args: CreateEdictTxArgs) -> Vec<u8> {
        let from_addr = Address::from_str(&args.from_address)
            .expect("failed to parse from address")
            .assume_checked();
        let to_addr = Address::from_str(&args.destination)
            .expect("failed to parse destination address")
            .assume_checked();
        let change_addr = args
            .change_address
            .map(|addr| {
                Address::from_str(&addr)
                    .expect("failed to parse change address")
                    .assume_checked()
            })
            .unwrap_or_else(|| from_addr.clone());

        let state = get_state();
        let index_provider = OrdIndexProvider::new(state.borrow().indexer_url());
        let runes_list = index_provider
            .get_rune_list()
            .await
            .expect("failed to get rune list");
        let rune_id = runes_list
            .into_iter()
            .find(|(_, spaced_rune, _)| args.rune_name == spaced_rune.to_string())
            .unwrap_or_else(|| panic!("rune {} is not in the list of runes", args.rune_name))
            .0;

        let utxo_provider = IcUtxoProvider::new(state.borrow().ic_btc_network());
        let input_utxos = utxo_provider
            .get_utxos(&from_addr)
            .await
            .expect("failed to get input utxos");
        let inputs = input_utxos
            .utxos
            .iter()
            .map(|utxo| TxInputInfo {
                outpoint: OutPoint {
                    txid: Txid::from_raw_hash(*Hash::from_bytes_ref(
                        &utxo.outpoint.txid.clone().try_into().expect("invalid txid"),
                    )),
                    vout: utxo.outpoint.vout,
                },
                tx_out: TxOut {
                    value: Amount::from_sat(utxo.value),
                    script_pubkey: from_addr.script_pubkey(),
                },
                derivation_path: DerivationPath::default(),
            })
            .collect();

        let fee_rate = utxo_provider
            .get_fee_rate()
            .await
            .expect("failed to get fee rate");

        let args = ord_rs::wallet::CreateEdictTxArgs {
            rune: rune_id,
            inputs,
            destination: to_addr,
            change_address: change_addr.clone(),
            rune_change_address: change_addr,
            amount: args.amount,
            fee_rate,
        };

        let builder = OrdTransactionBuilder::new(
            state.borrow().public_key(),
            ScriptType::P2WSH,
            state.borrow().wallet(),
        );
        let unsigned_tx = builder
            .create_edict_transaction(&args)
            .map_err(|err| {
                log::warn!("Failed to create withdraw transaction: {err:?}");
                WithdrawError::TransactionCreation
            })
            .expect("failed to create transaction");

        let mut bytes = vec![];
        unsigned_tx
            .consensus_encode(&mut bytes)
            .expect("failed to encode transaction");

        bytes
    }

    pub fn idl() -> Idl {
        generate_idl!()
    }
}

pub fn eth_address_to_subaccount(eth_address: &H160) -> Subaccount {
    let mut subaccount = [0; 32];
    subaccount[0..eth_address.0 .0.len()].copy_from_slice(eth_address.0.as_bytes());

    Subaccount(subaccount)
}

fn log_task_execution_error(task: InnerScheduledTask<RuneBridgeTask>) {
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

impl Metrics for RuneBridge {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

thread_local! {
    pub static STATE: Rc<RefCell<State>> = Rc::default();

    pub static SCHEDULER: Rc<RefCell<PersistentScheduler>> = Rc::new(RefCell::new({
        let pending_tasks =
            TasksStorage::new(MEMORY_MANAGER.with(|mm| mm.get(PENDING_TASKS_MEMORY_ID)));
            PersistentScheduler::new(pending_tasks)
    }));
}

pub(crate) fn get_state() -> Rc<RefCell<State>> {
    STATE.with(|state| state.clone())
}

pub(crate) fn get_scheduler() -> Rc<RefCell<PersistentScheduler>> {
    SCHEDULER.with(|scheduler| scheduler.clone())
}

pub(crate) fn get_operations_store() -> RuneOperationStore {
    let operations_memory = MEMORY_MANAGER.with(|mm| mm.get(OPERATIONS_MEMORY_ID));
    let operations_log_memory = MEMORY_MANAGER.with(|mm| mm.get(OPERATIONS_LOG_MEMORY_ID));
    let operations_map_memory = MEMORY_MANAGER.with(|mm| mm.get(OPERATIONS_MAP_MEMORY_ID));
    MinterOperationStore::with_memory(
        operations_memory,
        operations_log_memory,
        operations_map_memory,
        None,
    )
}
