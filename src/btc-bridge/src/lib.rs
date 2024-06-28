pub mod burn_request_store;
pub mod canister;
pub mod ck_btc_interface;
pub mod interface;
pub mod memory;
pub mod ops;
pub mod orders_store;
pub mod scheduler;
pub mod state;

use ic_metrics::Metrics;

pub use crate::canister::BtcBridge;

const EVM_INFO_INITIALIZATION_RETRIES: u32 = 5;
const EVM_INFO_INITIALIZATION_RETRY_DELAY_SEC: u32 = 2;
const EVM_INFO_INITIALIZATION_RETRY_MULTIPLIER: u32 = 2;

const MAINNET_CHAIN_ID: u32 = 0;
const TESTNET_CHAIN_ID: u32 = 1;
const REGTEST_CHAIN_ID: u32 = 2;

/// A marker to identify the canister as the BTC bridge canister.
#[no_mangle]
pub static BTC_BRIDGE_CANISTER_MARKER: &str = "BTC_BRIDGE_CANISTER";

pub fn idl() -> String {
    let btc_bridge_idl = BtcBridge::idl();
    let mut metrics_idl = <BtcBridge as Metrics>::get_idl();
    metrics_idl.merge(&btc_bridge_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
