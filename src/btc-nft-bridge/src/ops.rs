use core::sync::atomic::Ordering;
use std::cell::RefCell;
use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Message;
use bitcoin::sighash::SighashCache;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, FeeRate, OutPoint, PublicKey, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};
use did::{H160, H256};
use eth_signer::sign_strategy::TransactionSigner;
use ic_exports::ic_cdk::api::management_canister::bitcoin::{
    BitcoinNetwork, GetUtxosResponse, Utxo,
};
use ic_stable_structures::CellStructure;
use inscriber::interface::ecdsa_api::EcdsaSigner;
use inscriber::interface::{bitcoin_api, ecdsa_api};
use inscriber::wallet::fees::estimate_transaction_fees;
use inscriber::wallet::CanisterWallet;
use minter_did::erc721_mint_order::{ERC721MintOrder, ERC721SignedMintOrder};
use minter_did::id256::Id256;
use ord_rs::inscription::iid::InscriptionId;
use ord_rs::wallet::ScriptType;

use crate::constant::NONCE;
use crate::interface::bridge_api::{BridgeError, NftInscribeStatus, NftMintError, NftMintStatus};
use crate::interface::get_deposit_address;
use crate::interface::store::NftInfo;
use crate::rpc::{self, reverse_txid_byte_order};
use crate::state::State;

/// Swap a BTC-NFT for an ERC721.
///
/// This burns a BTC-NTF and mints an equivalent ERC721.
pub async fn nft_to_erc721(
    state: &RefCell<State>,
    eth_address: H160,
    nft_id: InscriptionId,
    holder_btc_addr: String,
) -> Result<NftMintStatus, NftMintError> {
    let nft = rpc::fetch_nft_token_details(state, nft_id, holder_btc_addr)
        .await
        .map_err(|e| NftMintError::InvalidNft(e.to_string()))?;

    let reveal_tx = rpc::fetch_reveal_transaction(state, &nft.id.0.txid)
        .await
        .map_err(|e| NftMintError::NftBridge(e.to_string()))?;

    rpc::parse_and_validate_inscription(reveal_tx, nft_id.index as usize)
        .await
        .map_err(|e| NftMintError::InvalidNft(e.to_string()))?;

    state.borrow_mut().inscriptions_mut().insert(nft);

    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);

    log::info!("Minting an ERC721 token with nft-id: {nft_id}");
    mint_erc721(state, eth_address, nft_id, nonce).await
}

pub async fn mint_erc721(
    state: &RefCell<State>,
    eth_address: H160,
    nft_id: InscriptionId,
    nonce: u32,
) -> Result<NftMintStatus, NftMintError> {
    let mint_order = prepare_mint_order(
        state,
        eth_address.clone(),
        nonce,
        nft_id.clone().to_string(),
    )
    .await?;
    store_mint_order(state, mint_order.clone(), &eth_address, nonce);

    Ok(match send_mint_order(state, mint_order.clone()).await {
        Ok(tx_id) => NftMintStatus::Minted {
            id: nft_id.into(),
            tx_id,
        },
        Err(err) => {
            log::warn!("Failed to send mint order: {err:?}");
            NftMintStatus::Signed(Box::new(mint_order))
        }
    })
}

async fn prepare_mint_order(
    state: &RefCell<State>,
    eth_address: H160,
    nonce: u32,
    token_uri: String,
) -> Result<ERC721SignedMintOrder, NftMintError> {
    log::info!("preparing mint order");

    let (signer, mint_order) = {
        let state_ref = state.borrow();

        let sender_chain_id = state_ref.btc_chain_id();
        let dst_token = state_ref.nft_token_address();
        let sender = Id256::from_evm_address(&eth_address, sender_chain_id);
        let src_token = Id256::from(&ic_exports::ic_kit::ic::id());

        let recipient_chain_id = state_ref.erc721_chain_id();

        let mint_order = ERC721MintOrder {
            sender,
            src_token,
            recipient: eth_address,
            dst_token,
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name: state_ref.token_name(),
            symbol: state_ref.token_symbol(),
            approve_spender: Default::default(),
            token_uri,
        };

        log::info!("mint order: {mint_order:?}");

        let signer = state_ref.signer().get().clone();

        (signer, mint_order)
    };

    let signed_mint_order = mint_order
        .encode_and_sign(&signer)
        .await
        .map_err(|err| NftMintError::Sign(format!("{err:?}")))?;

    Ok(signed_mint_order)
}

fn store_mint_order(
    state: &RefCell<State>,
    signed_mint_order: ERC721SignedMintOrder,
    eth_address: &H160,
    nonce: u32,
) {
    let mut state = state.borrow_mut();
    let sender_chain_id = state.btc_chain_id();
    let sender = Id256::from_evm_address(eth_address, sender_chain_id);
    state
        .mint_orders_mut()
        .push(sender, nonce, signed_mint_order);

    log::trace!("Mint order added");
}

async fn send_mint_order(
    state: &RefCell<State>,
    mint_order: ERC721SignedMintOrder,
) -> Result<H256, NftMintError> {
    log::info!("Sending mint transaction");

    let signer = state.borrow().signer().get().clone();
    let sender = signer
        .get_address()
        .await
        .map_err(|err| NftMintError::Sign(format!("{err:?}")))?;

    let (evm_info, evm_params) = {
        let state = state.borrow();

        let evm_info = state.get_evm_info();
        let evm_params = state
            .get_evm_params()
            .clone()
            .ok_or(NftMintError::NotInitialized(
                "Bridge must be initialized first".to_string(),
            ))?;

        (evm_info, evm_params)
    };

    let mut tx = minter_contract_utils::erc721_bridge_api::mint_transaction(
        sender.0,
        evm_info.bridge_contract.0,
        evm_params.nonce.into(),
        evm_params.gas_price.into(),
        mint_order.0.to_vec(),
        evm_params.chain_id as _,
    );

    let signature = signer
        .sign_transaction(&(&tx).into())
        .await
        .map_err(|err| NftMintError::Sign(format!("{err:?}")))?;

    tx.r = signature.r.0;
    tx.s = signature.s.0;
    tx.v = signature.v.0;
    tx.hash = tx.hash();

    let client = evm_info.link.get_json_rpc_client();
    let id = client
        .send_raw_transaction(tx)
        .await
        .map_err(|err| NftMintError::Evm(format!("{err:?}")))?;

    state.borrow_mut().update_evm_params(|p| {
        if let Some(params) = p.as_mut() {
            params.nonce += 1;
        }
    });

    log::info!("Mint transaction sent: {id}");

    Ok(id.into())
}

/// Swap an NFT on Eth for a BTC-NFT.
///
/// This burns an Nft and inscribes an equivalent on BTC.
pub async fn erc721_to_nft(
    state: &RefCell<State>,
    request_id: u32,
    nft_id: InscriptionId,
    dst_addr: &str,
    fee_canister_address: String,
    eth_address: &H160,
) -> Result<NftInscribeStatus, BridgeError> {
    let network = state.borrow().ic_btc_network();

    let bridge_addr = get_deposit_address(state, eth_address, network).await;

    let nft_info = rpc::fetch_nft_token_details(state, nft_id, bridge_addr)
        .await
        .map_err(|e| NftMintError::InvalidNft(e.to_string()))?;

    let tx_id = withdraw_nft(
        state,
        request_id,
        nft_info,
        dst_addr,
        fee_canister_address,
        eth_address,
    )
    .await
    .map_err(|e| BridgeError::Erc721Burn(e.to_string()))?;

    Ok(NftInscribeStatus {
        tx_id: tx_id.to_string(),
    })
}

async fn withdraw_nft(
    state: &RefCell<State>,
    request_id: u32,
    nft: NftInfo,
    dst_addr: &str,
    fee_canister_address: String,
    eth_address: &H160,
) -> Result<Txid, BridgeError> {
    let tx_id = nft.id.0.txid.to_string();
    if !state.borrow().has_nft(&tx_id) {
        return Err(BridgeError::Erc721Burn(format!(
            "Specified tx ID ({}) not associated with any BTC inscription",
            tx_id
        )));
    }

    let utxo = rpc::fetch_nft_utxo(state, &tx_id, eth_address)
        .await
        .map_err(|e| BridgeError::GetTransactionById(e.to_string()))?;

    let network = state.borrow().ic_btc_network();

    log::info!(
        "Transferring requested NFT to {} with request id {}",
        dst_addr,
        request_id
    );

    let btc_network = CanisterWallet::map_network(network);

    let dst_addr = Address::from_str(dst_addr)
        .map_err(|e| BridgeError::MalformedAddress(e.to_string()))?
        .require_network(btc_network)
        .map_err(|e| BridgeError::MalformedAddress(e.to_string()))?;

    let leftovers_addr = Address::from_str(&fee_canister_address)
        .map_err(|e| BridgeError::MalformedAddress(e.to_string()))?
        .require_network(btc_network)
        .map_err(|e| BridgeError::MalformedAddress(e.to_string()))?;

    // get utxos for fees
    let signer = state.borrow().ecdsa_signer();
    let wallet = CanisterWallet::new(network, signer);
    let fee_rate = wallet.get_fee_rate().await;
    let utxos = bitcoin_api::get_utxos(network, fee_canister_address)
        .await
        .map_err(|e| BridgeError::GetUtxos(e.to_string()))?;

    let outputs = vec![
        TxOut {
            value: Amount::ONE_SAT,
            script_pubkey: dst_addr.script_pubkey(),
        },
        TxOut {
            value: Amount::ONE_SAT,
            script_pubkey: leftovers_addr.script_pubkey(),
        },
    ];

    let (fee_utxos, fee_amount) = match find_fee_utxos(utxos, &fee_rate, &outputs) {
        None => return Err(BridgeError::Erc721Burn("Insufficient funds".to_string())),
        Some(utxos) => utxos,
    };

    // transfer the UTXO to the destination address
    let signer = state.borrow().ecdsa_signer();
    let result = transfer_utxo(
        &utxo,
        &fee_utxos,
        fee_amount,
        leftovers_addr,
        dst_addr,
        network,
        signer,
    )
    .await
    .map_err(|e| BridgeError::Erc721Burn(e.to_string()));

    let mut state = state.borrow_mut();
    if result.is_ok() {
        state
            .inscriptions_mut()
            .remove(nft.id.0.txid.to_string())
            .map_err(|e| BridgeError::Erc721Burn(e.to_string()))?;

        state.burn_requests_mut().set_transferred(request_id);
        state.burn_requests_mut().remove(request_id);
    } else {
        log::error!("Failed to transfer BRC20 for request {}", request_id);
    }

    result
}

fn find_fee_utxos(
    res: GetUtxosResponse,
    fee_rate: &FeeRate,
    outputs: &[TxOut],
) -> Option<(Vec<Utxo>, Amount)> {
    let mut tx_inputs = 2;
    loop {
        let fee = estimate_transaction_fees(
            ScriptType::P2WSH,
            tx_inputs,
            fee_rate,
            &None,
            outputs.to_vec(),
        );
        // sort utxos by value
        let mut utxos = res.utxos.clone();
        utxos.sort_by_key(|u| u.value);
        // find `tx_inputs - 1` outpoints to satisfy fee
        let mut outpoints = vec![];
        let mut total_value = 0;
        for _ in 0..(tx_inputs - 1) {
            let next = match utxos.pop() {
                Some(u) => u,
                None => return None,
            };
            total_value += next.value;
            outpoints.push(next);
        }
        // check if value is satisfied
        if total_value >= fee.to_sat() {
            return Some((outpoints, fee));
        } else {
            // try with more inputs
            tx_inputs += 1;
        }
    }
}

async fn transfer_utxo(
    utxo: &Utxo,
    fee_utxos: &[Utxo],
    fee_amount: Amount,
    leftovers_address: Address,
    dst_address: Address,
    network: BitcoinNetwork,
    signer: EcdsaSigner,
) -> Result<Txid, BridgeError> {
    let leftovers_amount =
        Amount::from_sat(fee_utxos.iter().map(|utxo| utxo.value).sum::<u64>()) - fee_amount;
    let input = [utxo]
        .into_iter()
        .chain(fee_utxos)
        .map(|utxo| {
            let txid = reverse_txid_byte_order(utxo);
            let mut outpoint = utxo.outpoint.clone();
            outpoint.txid = txid;
            outpoint
        })
        .map(|outpoint| TxIn {
            previous_output: OutPoint {
                txid: Txid::from_slice(&outpoint.txid).expect("bad txid"),
                vout: outpoint.vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::from_consensus(0xffffffff),
            witness: Witness::new(),
        })
        .collect::<Vec<TxIn>>();

    let mut output = vec![TxOut {
        value: Amount::from_sat(utxo.value),
        script_pubkey: dst_address.script_pubkey(),
    }];
    if leftovers_amount > Amount::ZERO {
        output.push(TxOut {
            value: leftovers_amount,
            script_pubkey: leftovers_address.script_pubkey(),
        });
    }

    let pubkey = signer.public_key();

    let unsigned_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input,
        output,
    };

    let signed_tx = sign_transaction(
        &unsigned_tx,
        &[utxo.clone()],
        leftovers_address,
        pubkey,
        signer,
    )
    .await?;

    bitcoin_api::send_transaction(network, bitcoin::consensus::serialize(&signed_tx)).await;

    Ok(signed_tx.txid())
}

async fn sign_transaction(
    unsigned_tx: &Transaction,
    utxos: &[Utxo],
    holder_btc_addr: Address,
    holder_btc_pubkey: PublicKey,
    signer: EcdsaSigner,
) -> Result<Transaction, BridgeError> {
    let mut hash = SighashCache::new(unsigned_tx.clone());
    for (index, input) in utxos.iter().enumerate() {
        let sighash = hash
            .p2wpkh_signature_hash(
                index,
                &holder_btc_addr.script_pubkey(),
                Amount::from_sat(input.value),
                bitcoin::EcdsaSighashType::All,
            )
            .map_err(|e| BridgeError::SignatureError(e.to_string()))?;

        log::debug!("Signing transaction and verifying signature");
        let signature = {
            // sign
            let msg = Message::from(sighash);
            let dp = ecdsa_api::get_btc_derivation_path(&H160::default())
                .map_err(|e| BridgeError::SignatureError(e.to_string()))?;

            use ord_rs::BtcTxSigner;
            let signature = signer
                .sign_with_ecdsa(msg, &dp)
                .await
                .map_err(|e| BridgeError::SignatureError(e.to_string()))?;
            signer
                .verify_ecdsa(&signature, &msg, &holder_btc_pubkey)
                .map_err(|e| BridgeError::SignatureError(e.to_string()))?;

            signature
        };

        log::debug!("signature: {}", signature.serialize_der());

        // append witness
        let signature = bitcoin::ecdsa::Signature::sighash_all(signature);
        let witness = Witness::p2wpkh(&signature, &holder_btc_pubkey.inner);
        *hash
            .witness_mut(index)
            .ok_or(BridgeError::NoSuchUtxo(index.to_string()))? = witness;
    }

    Ok(hash.into_transaction())
}
