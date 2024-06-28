pub mod canister;
pub mod memory;
pub mod operation;
pub mod state;
pub mod tasks;

use ic_metrics::Metrics;

pub use crate::canister::EvmMinter;

/// A marker to identify the canister as the ICRC bridge canister.
#[no_mangle]
pub static ERC20_CANISTER_MARKER: &str = "ERC20_BRIDGE_CANISTER";

pub fn idl() -> String {
    let signature_verification_idl = EvmMinter::idl();
    let mut metrics_idl = <EvmMinter as Metrics>::get_idl();
    metrics_idl.merge(&signature_verification_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
