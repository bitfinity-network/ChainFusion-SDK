use ic_exports::ic_cdk::api::management_canister::bitcoin::{
    self as IcBtc, BitcoinNetwork, GetBalanceRequest, GetCurrentFeePercentilesRequest,
    GetUtxosRequest, GetUtxosResponse, MillisatoshiPerByte, SendTransactionRequest, Utxo,
    UtxoFilter,
};

use crate::constant::UTXO_MIN_CONFIRMATION;

/// Returns the balance of the given bitcoin address.
///
/// NOTE: Relies on the `bitcoin_get_balance` endpoint.
/// See https://internetcomputer.org/docs/current/references/ic-interface-spec/#ic-bitcoin_get_balance
pub async fn get_balance(network: BitcoinNetwork, address: String) -> u64 {
    IcBtc::bitcoin_get_balance(GetBalanceRequest {
        address,
        network,
        min_confirmations: None,
    })
    .await
    .expect("Failed to retrieve balance for specified address")
    .0
}

/// Fetches all UTXOs for the given address using pagination.
///
/// NOTE: Relies on the `bitcoin_get_utxos` endpoint.
/// See https://internetcomputer.org/docs/current/references/ic-interface-spec/#ic-bitcoin_get_utxos
pub async fn get_utxos(
    network: BitcoinNetwork,
    address: String,
) -> Result<GetUtxosResponse, String> {
    let mut all_utxos = Vec::<Utxo>::new();
    let mut page_filter: Option<UtxoFilter> =
        Some(UtxoFilter::MinConfirmations(UTXO_MIN_CONFIRMATION));
    let mut tip_block_hash = Vec::<u8>::new();
    let mut tip_height = 0u32;

    let mut last_page: Option<Vec<u8>>;

    loop {
        let get_utxos_request = GetUtxosRequest {
            address: address.clone(),
            network,
            filter: page_filter,
        };

        let utxos_res = IcBtc::bitcoin_get_utxos(get_utxos_request).await;

        match utxos_res {
            Ok(response) => {
                let (get_utxos_response,) = response;
                all_utxos.extend(get_utxos_response.utxos);
                // Update tip_block_hash and tip_height only if they are not already set
                if tip_block_hash.is_empty() {
                    tip_block_hash = get_utxos_response.tip_block_hash;
                }
                if tip_height == 0 {
                    tip_height = get_utxos_response.tip_height;
                }
                last_page = get_utxos_response.next_page.clone();
                page_filter = last_page.clone().map(UtxoFilter::Page);
            }
            Err(e) => return Err(format!("{:?}", e)),
        }

        if page_filter.is_none() {
            break;
        }
    }

    Ok(GetUtxosResponse {
        utxos: all_utxos,
        tip_block_hash,
        tip_height,
        next_page: last_page,
    })
}

/// Returns the 100 fee percentiles measured in millisatoshi/byte.
///
/// Percentiles are computed from the last 10,000 transactions (if available).
///
/// NOTE: Relies on the `bitcoin_get_current_fee_percentiles` endpoint.
/// See https://internetcomputer.org/docs/current/references/ic-interface-spec/#ic-bitcoin_get_current_fee_percentiles
pub async fn get_current_fee_percentiles(network: BitcoinNetwork) -> Vec<MillisatoshiPerByte> {
    IcBtc::bitcoin_get_current_fee_percentiles(GetCurrentFeePercentilesRequest { network })
        .await
        .expect("Failed to retrieve current fee percentiles")
        .0
}

/// Sends a (signed) transaction to the bitcoin network.
///
/// NOTE: Relies on the `bitcoin_send_transaction` endpoint.
/// See https://internetcomputer.org/docs/current/references/ic-interface-spec/#ic-bitcoin_send_transaction
pub async fn send_transaction(network: BitcoinNetwork, transaction: Vec<u8>) {
    IcBtc::bitcoin_send_transaction(SendTransactionRequest {
        network,
        transaction,
    })
    .await
    .expect("Failed to send transaction");
}
