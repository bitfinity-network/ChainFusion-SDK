mod build_data;
mod canister;
mod constant;
mod interface;
mod memory;
mod ops;
mod rpc;
mod scheduler;
mod state;

use canister::NftBridge;
use ic_metrics::Metrics;

pub fn idl() -> String {
    let btc_bridge_idl = NftBridge::idl();
    let mut metrics_idl = <NftBridge as Metrics>::get_idl();
    metrics_idl.merge(&btc_bridge_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
