use std::marker::PhantomData;
use std::rc::Rc;

use ic_metrics::Metrics;
pub use state::SigningStrategy;

pub(crate) use crate::canister::MinterCanister;

mod build_data;
pub mod canister;
mod constant;
mod memory;
pub mod operation;
pub mod state;
mod tasks;
pub mod tokens;

type ForceNotSendAndNotSync = PhantomData<Rc<()>>;

/// A marker to identify the canister as the ICRC bridge canister.
#[no_mangle]
pub static ICRC_CANISTER_MARKER: &str = "ICRC_BRIDGE_CANISTER";

pub fn idl() -> String {
    let minter_canister_idl = MinterCanister::idl();

    let mut metrics_idl = <MinterCanister as Metrics>::get_idl();
    metrics_idl.merge(&minter_canister_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
