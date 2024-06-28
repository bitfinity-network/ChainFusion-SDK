//! A module for interacting with the management canister.
//!
//! This module provides a set of functions for interacting with the management canister.

use candid::{CandidType, Principal};
use ic_exports::ic_cdk::api::call::{call, call_with_payment128};
use ic_exports::ic_cdk::api::management_canister::main::{
    CanisterStatusResponse, CreateCanisterArgument, InstallCodeArgument,
};
use ic_exports::ic_cdk::api::management_canister::provisional::CanisterIdRecord;
use serde::de::DeserializeOwned;

use crate::error::Result;

pub type CanisterId = Principal;

pub enum CallCycles {
    Cycles(u128),
    Free,
}

pub const MANAGEMENT_CANISTER_ID: Principal = Principal::management_canister();

pub struct Management;

impl Management {
    pub async fn call<A, R>(method: &str, args: A, cycles: CallCycles) -> Result<R>
    where
        A: CandidType,
        R: CandidType + DeserializeOwned,
    {
        let res: Result<(R,), _> = match cycles {
            CallCycles::Cycles(cycles) => {
                call_with_payment128(MANAGEMENT_CANISTER_ID, method, (args,), cycles).await
            }
            CallCycles::Free => call(MANAGEMENT_CANISTER_ID, method, (args,)).await,
        };

        match res {
            Ok((res,)) => Ok(res),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn create_canister(
        arg: CreateCanisterArgument,
        cycles: u128,
    ) -> Result<CanisterIdRecord> {
        Management::call("create_canister", arg, CallCycles::Cycles(cycles)).await
    }

    pub async fn install_code(arg: InstallCodeArgument) -> Result {
        Management::call("install_code", arg, CallCycles::Free).await
    }

    pub async fn canister_status(canister_id: CanisterId) -> Result<CanisterStatusResponse> {
        let arg = CanisterIdRecord { canister_id };

        Management::call("canister_status", arg, CallCycles::Free).await
    }

    pub async fn start_canister(canister_id: CanisterId) -> Result {
        let arg = CanisterIdRecord { canister_id };

        Management::call("start_canister", arg, CallCycles::Free).await
    }

    pub async fn stop_canister(canister_id: CanisterId) -> Result {
        let arg = CanisterIdRecord { canister_id };

        Management::call("stop_canister", arg, CallCycles::Free).await
    }

    pub async fn delete_canister(canister_id: CanisterId) -> Result {
        let arg = CanisterIdRecord { canister_id };

        Management::call("delete_canister", arg, CallCycles::Free).await
    }

    pub async fn uninstall_code(canister_id: CanisterId) -> Result {
        let arg = CanisterIdRecord { canister_id };

        Management::call("uninstall_code", arg, CallCycles::Free).await
    }
}
