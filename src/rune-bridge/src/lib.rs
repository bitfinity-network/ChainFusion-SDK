pub mod canister;
pub mod core;
pub mod interface;
pub mod key;
pub mod ledger;
pub mod memory;
pub mod operation;
pub mod rune_info;
pub mod scheduler;
pub mod state;
pub mod task;

use ic_metrics::Metrics;

pub use crate::canister::RuneBridge;

const EVM_INFO_INITIALIZATION_RETRIES: u32 = 5;
const EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC: u32 = 2;
const EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER: u32 = 2;

const MAINNET_CHAIN_ID: u32 = 0;
const TESTNET_CHAIN_ID: u32 = 1;
const REGTEST_CHAIN_ID: u32 = 2;

/// A marker to identify the canister as the RUNE bridge canister.
#[no_mangle]
pub static RUNE_BRIDGE_CANISTER_MARKER: &str = "RUNE_BRIDGE_CANISTER";

pub fn idl() -> String {
    let rune_bridge_idl = RuneBridge::idl();
    let mut metrics_idl = <RuneBridge as Metrics>::get_idl();
    metrics_idl.merge(&rune_bridge_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
