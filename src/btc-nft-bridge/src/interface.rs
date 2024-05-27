use std::cell::RefCell;

use did::H160;
use ic_exports::ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;
use inscriber::wallet::CanisterWallet;

use crate::state::State;

pub mod bridge_api;
pub mod store;

/// Retrieves the Bitcoin address for the given derivation path.
pub(crate) async fn get_deposit_address(
    state: &RefCell<State>,
    eth_address: &H160,
    network: BitcoinNetwork,
) -> String {
    let ecdsa_signer = { state.borrow().ecdsa_signer() };
    CanisterWallet::new(network, ecdsa_signer)
        .get_bitcoin_address(eth_address)
        .await
        .to_string()
}
