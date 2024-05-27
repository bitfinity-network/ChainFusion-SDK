use std::cell::RefCell;
use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
};
use did::H160;
use futures::TryFutureExt;
use ic_exports::ic_cdk::api::management_canister::bitcoin::{
    BitcoinNetwork, GetUtxosResponse, Utxo,
};
use ic_exports::ic_cdk::api::management_canister::http_request::{
    http_request, CanisterHttpRequestArgument, HttpHeader, HttpMethod,
};
use inscriber::constant::UTXO_MIN_CONFIRMATION;
use inscriber::interface::bitcoin_api;
use ord_rs::inscription::iid::InscriptionId;
use ord_rs::{Nft, OrdParser};
use serde::Deserialize;

use crate::constant::{
    HTTP_OUTCALL_MAX_RESPONSE_BYTES, HTTP_OUTCALL_PER_CALL_COST, HTTP_OUTCALL_REQ_PER_BYTE_COST,
    HTTP_OUTCALL_RES_DEFAULT_SIZE, HTTP_OUTCALL_RES_PER_BYTE_COST, MAX_HTTP_RESPONSE_BYTES,
};
use crate::interface::bridge_api::{BridgeError, DepositError};
use crate::interface::get_deposit_address;
use crate::interface::store::NftInfo;
use crate::state::State;

/// Retrieves and validates the details of a NFT token given its ticker.
pub async fn fetch_nft_token_details(
    state: &RefCell<State>,
    id: InscriptionId,
    holder: String,
) -> anyhow::Result<NftInfo> {
    let (network, ord_url) = {
        let state = state.borrow();
        (state.btc_network(), state.ord_url())
    };

    // check that BTC address is valid and/or
    // corresponds to the network.
    is_valid_btc_address(&holder, network)?;

    // <https://docs.ordinals.com/guides/explorer.html#json-api>
    let url = format!("{ord_url}/inscription/{id}");

    log::info!("Retrieving inscriptions for {holder} from: {url}");

    let request_params = CanisterHttpRequestArgument {
        url,
        max_response_bytes: Some(MAX_HTTP_RESPONSE_BYTES),
        method: HttpMethod::GET,
        headers: vec![HttpHeader {
            name: "Accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
        transform: None,
    };

    let cycles = get_estimated_http_outcall_cycles(&request_params);
    let result = http_request(request_params, cycles)
        .await
        .map_err(|err| BridgeError::FetchNftTokenDetails(format!("{err:?}")))?
        .0;

    if result.status.to_string() != "200" {
        log::error!("Failed to fetch data: HTTP status {}", result.status);
        return Err(BridgeError::FetchNftTokenDetails("Failed to fetch data".to_string()).into());
    }

    log::info!(
        "Response from indexer: Status: {} Body: {}",
        result.status,
        String::from_utf8_lossy(&result.body)
    );

    let inscription: InscriptionResponse = serde_json::from_slice(&result.body).map_err(|err| {
        log::error!("Failed to retrieve inscriptions details from the indexer: {err:?}");
        BridgeError::FetchNftTokenDetails(format!("{err:?}"))
    })?;

    let nft_id = InscriptionId::from_str(&inscription.id).map_err(|err| {
        log::error!("Failed to parse NFT ID: {err:?}");
        BridgeError::FetchNftTokenDetails(format!("{err:?}"))
    })?;

    Ok(NftInfo::new(
        nft_id.into(),
        inscription.address,
        inscription.satpoint,
    )?)
}

/// Retrieves (and re-constructs) the reveal transaction by its ID.
///
/// We use the reveal transaction (as opposed to the commit transaction)
/// because it contains the actual NFT inscription that needs to be parsed.
pub(crate) async fn fetch_reveal_transaction(
    state: &RefCell<State>,
    reveal_tx_id: &Txid,
) -> anyhow::Result<Transaction> {
    let ord_url: String = state.borrow().ord_url();
    Ok(get_nft_transaction_by_id(&ord_url, reveal_tx_id)
        .await
        .map_err(|e| BridgeError::GetTransactionById(e.to_string()))?)
}

/// Retrieves (and re-constructs) the reveal transaction by its ID.
///
/// We use the reveal transaction (as opposed to the commit transaction)
/// because it contains the actual NFT inscription that needs to be parsed.
pub(crate) async fn fetch_nft_utxo(
    state: &RefCell<State>,
    reveal_tx_id: &str,
    eth_address: &H160,
) -> anyhow::Result<Utxo> {
    let ic_btc_network = state.borrow().ic_btc_network();

    let bridge_addr = get_deposit_address(state, eth_address, ic_btc_network).await;

    let nft_utxo = find_inscription_utxo(
        ic_btc_network,
        bridge_addr,
        hex::decode(reveal_tx_id).expect("failed to decode reveal_tx_id"),
    )
    .map_err(|e| BridgeError::FindInscriptionUtxo(e.to_string()))
    .await?;
    Ok(nft_utxo)
}

pub(crate) async fn parse_and_validate_inscription(
    reveal_tx: Transaction,
    index: usize,
) -> Result<Nft, BridgeError> {
    log::info!("Parsing NFT inscription from transaction");

    let (_, parser) = OrdParser::parse_one(&reveal_tx, index)
        .map_err(|e| BridgeError::InscriptionParsing(e.to_string()))?;

    match parser {
        OrdParser::Ordinal(nft) => Ok(nft),
        _ => Err(BridgeError::InscriptionParsing(
            "Invalid inscription".to_string(),
        )),
    }
}

fn network_as_str(network: Network) -> &'static str {
    match network {
        Network::Testnet => "/testnet",
        Network::Regtest => "/regtest",
        Network::Signet => "/signet",
        _ => "",
    }
}

async fn get_nft_transaction_by_id(ord_url: &str, txid: &Txid) -> anyhow::Result<Transaction> {
    let url = format!("{ord_url}/tx/{txid}");

    let request_params = CanisterHttpRequestArgument {
        url,
        max_response_bytes: Some(HTTP_OUTCALL_MAX_RESPONSE_BYTES),
        method: HttpMethod::GET,
        headers: vec![HttpHeader {
            name: "Accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
        transform: None,
    };

    let cycles = get_estimated_http_outcall_cycles(&request_params);

    let response = http_request(request_params, cycles)
        .await
        .map_err(|err| BridgeError::FetchNftTokenDetails(format!("{err:?}")))?
        .0;

    log::info!(
        "Indexer responded with: STATUS: {} HEADERS: {:?} BODY: {}",
        response.status,
        response.headers,
        String::from_utf8_lossy(&response.body)
    );

    let tx_html: TransactionHtml = serde_json::from_slice(&response.body).map_err(|err| {
        log::error!("Failed to retrieve the reveal transaction from the indexer: {err:?}");
        BridgeError::GetTransactionById(format!("{err:?}"))
    })?;

    Ok(tx_html.transaction)
}

fn is_valid_btc_address(addr: &str, network: Network) -> Result<bool, BridgeError> {
    let network_str = network_as_str(network);

    if !Address::from_str(addr)
        .expect("Failed to convert to bitcoin address")
        .is_valid_for_network(network)
    {
        log::error!("The given bitcoin address {addr} is not valid for {network_str}");
        return Err(BridgeError::MalformedAddress(addr.to_string()));
    }

    Ok(true)
}

/// Validates a reveal transaction ID by checking if it matches
/// the transaction ID of the received UTXO.
async fn find_inscription_utxo(
    network: BitcoinNetwork,
    deposit_addr: String,
    txid: Vec<u8>,
) -> Result<Utxo, BridgeError> {
    let utxos_response = bitcoin_api::get_utxos(network, deposit_addr.clone())
        .await
        .map_err(|e| BridgeError::GetTransactionById(e.to_string()))?;

    let utxos = validate_utxos(utxos_response)
        .map_err(|err| BridgeError::FindInscriptionUtxo(format!("{err:?}")))?;

    let nft_utxo = utxos
        .iter()
        .find(|utxo| reverse_txid_byte_order(utxo) == txid)
        .cloned();

    match nft_utxo {
        Some(utxo) => Ok(utxo),
        None => Err(BridgeError::GetTransactionById(
            format!("No matching UTXO found: {}", hex::encode(txid),).to_string(),
        )),
    }
}

fn validate_utxos(utxo_response: GetUtxosResponse) -> Result<Vec<Utxo>, DepositError> {
    let min_confirmations = UTXO_MIN_CONFIRMATION;
    let current_confirmations = utxo_response
        .utxos
        .iter()
        .map(|utxo| utxo_response.tip_height - utxo.height + 1)
        .min()
        .unwrap_or_default();

    if min_confirmations > current_confirmations {
        Err(DepositError::Pending {
            min_confirmations,
            current_confirmations,
        })
    } else {
        Ok(utxo_response.utxos)
    }
}

/// Reverses the byte order of the transaction ID.
///
/// The IC management canister returns bytes of txid in reversed order,
/// so we need to undo the operation first before before consuming the output.
pub fn reverse_txid_byte_order(utxo: &Utxo) -> Vec<u8> {
    utxo.outpoint
        .txid
        .iter()
        .copied()
        .rev()
        .collect::<Vec<u8>>()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InscriptionResponse {
    id: String,
    address: String,
    satpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct TxInfo {
    version: i32,
    locktime: u32,
    vin: Vec<Vin>,
    vout: Vec<Vout>,
}

impl TryFrom<TxInfo> for Transaction {
    type Error = anyhow::Error;

    fn try_from(info: TxInfo) -> Result<Self, Self::Error> {
        let version = Version(info.version);
        let lock_time = LockTime::from_consensus(info.locktime);

        let mut tx_in = Vec::with_capacity(info.vin.len());
        for input in info.vin {
            let txid = Txid::from_str(&input.txid)?;
            let vout = input.vout;
            let script_sig = ScriptBuf::from_hex(&input.prevout.scriptpubkey)?;

            let mut witness = Witness::new();
            for item in input.witness {
                witness.push(ScriptBuf::from_hex(&item)?);
            }

            let tx_input = TxIn {
                previous_output: OutPoint { txid, vout },
                script_sig,
                sequence: Sequence(input.sequence),
                witness,
            };

            tx_in.push(tx_input);
        }

        let mut tx_out = Vec::with_capacity(info.vout.len());
        for output in info.vout {
            let script_pubkey = ScriptBuf::from_hex(&output.scriptpubkey)?;
            let value = Amount::from_sat(output.value);

            tx_out.push(TxOut {
                script_pubkey,
                value,
            });
        }

        Ok(Transaction {
            version,
            lock_time,
            input: tx_in,
            output: tx_out,
        })
    }
}

fn get_estimated_http_outcall_cycles(req: &CanisterHttpRequestArgument) -> u128 {
    let headers_size = req.headers.iter().fold(0u128, |len, header| {
        len + header.value.len() as u128 + header.name.len() as u128
    });

    let mut request_size = req.url.len() as u128 + headers_size;

    if let Some(transform) = &req.transform {
        request_size += transform.context.len() as u128;
    }

    if let Some(body) = &req.body {
        request_size += body.len() as u128;
    }

    let http_outcall_cost: u128 = HTTP_OUTCALL_PER_CALL_COST
        + HTTP_OUTCALL_REQ_PER_BYTE_COST * request_size
        + HTTP_OUTCALL_RES_PER_BYTE_COST
            * req
                .max_response_bytes
                .unwrap_or(HTTP_OUTCALL_RES_DEFAULT_SIZE) as u128;

    http_outcall_cost
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Vin {
    txid: String,
    vout: u32,
    sequence: u32,
    is_coinbase: bool,
    prevout: Prevout,
    witness: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Prevout {
    scriptpubkey: String,
    scriptpubkey_asm: String,
    scriptpubkey_type: String,
    scriptpubkey_address: Option<String>,
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Vout {
    scriptpubkey: String,
    scriptpubkey_asm: String,
    scriptpubkey_type: String,
    scriptpubkey_address: Option<String>,
    value: u64,
}

#[derive(Debug, PartialEq, Deserialize)]
struct TransactionHtml {
    inscription_count: u32,
    transaction: Transaction,
    txid: Txid,
}
