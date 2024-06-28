#![allow(clippy::upper_case_acronyms)]

use canister::CanisterFactory;

pub mod canister;
mod error;
mod hash;
mod management;
mod memory;
mod state;
pub(crate) mod types;

pub fn idl() -> String {
    let factory_idl = CanisterFactory::idl();

    let mut metrics_idl = <CanisterFactory as ic_metrics::Metrics>::get_idl();
    metrics_idl.merge(&factory_idl);

    candid::pretty::candid::compile(&metrics_idl.env.env, &Some(metrics_idl.actor))
}
