use candid::Principal;
use did::H160;
use ic_exports::ic_cdk::{self, api};
use ic_exports::ic_cdk_macros::inspect_message;
use ic_exports::ic_kit::ic;
use minter_did::error::Result;
use minter_did::reason::Icrc2Burn;

use crate::state::State;
use crate::MinterCanister;

#[inspect_message]
async fn inspect_message() {
    let check_result = inspect_method(&api::call::method_name()).await;

    if let Err(e) = check_result {
        ic::trap(&format!("Call rejected by inspect check: {e:?}"));
    } else {
        api::call::accept_message();
    }
}

async fn inspect_method(method: &str) -> Result<()> {
    let state = State::default();

    match method {
        "set_logger_filter" => {
            MinterCanister::set_logger_filter_inspect_message_check(ic::caller(), &state)
        }
        "ic_logs" => MinterCanister::ic_logs_inspect_message_check(ic::caller(), &state),
        "set_evm_principal" => {
            let (evm,) = api::call::arg_data::<(Principal,)>();
            MinterCanister::set_evm_principal_inspect_message_check(ic::caller(), evm, &state)
        }
        "set_owner" => {
            let (owner,) = api::call::arg_data::<(Principal,)>();
            MinterCanister::set_owner_inspect_message_check(ic::caller(), owner, &state)
        }
        "burn_icrc2" => {
            let (reason,) = api::call::arg_data::<(Icrc2Burn,)>();
            MinterCanister::burn_icrc2_inspect_message_check(&reason)
        }
        "register_evmc_bft_bridge" => {
            let (bft_bridge_address,) = api::call::arg_data::<(H160,)>();
            MinterCanister::register_evmc_bft_bridge_inspect_message_check(
                ic::caller(),
                bft_bridge_address,
                &state,
            )
        }
        _ => Ok(()),
    }
}
