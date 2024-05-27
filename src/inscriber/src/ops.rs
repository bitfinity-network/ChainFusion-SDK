use std::str::FromStr as _;

use bitcoin::{Address, Txid};
use did::H160;
use ic_exports::ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;
use ord_rs::MultisigConfig;

use crate::interface::ecdsa_api::EcdsaSigner;
use crate::interface::{
    Brc20TransferTransactions, InscribeError, InscribeResult, InscribeTransactions,
    InscriptionFees, Multisig, Protocol,
};
use crate::wallet::CanisterWallet;

/// Inscribes a message onto the Bitcoin blockchain using the given inscription
/// type.
pub async fn inscribe(
    inscription_type: Protocol,
    inscription: String,
    leftovers_address: String,
    dst_address: String,
    multisig_config: Option<Multisig>,
    signer: EcdsaSigner,
    network: BitcoinNetwork,
) -> InscribeResult<InscribeTransactions> {
    let wallet = CanisterWallet::new(network, signer);

    let leftovers_address = get_address(leftovers_address, network)?;
    let dst_address = get_address(dst_address, network)?;

    let multisig_config = multisig_config.map(|m| MultisigConfig {
        required: m.required,
        total: m.total,
    });

    wallet
        .inscribe(
            inscription_type,
            inscription,
            dst_address,
            leftovers_address,
            multisig_config,
        )
        .await
}

/// Inscribes and sends the inscribed sat from this canister to the given address.
pub async fn brc20_transfer(
    inscription: String,
    leftovers_address: String,
    dst_address: String,
    multisig_config: Option<Multisig>,
    signer: EcdsaSigner,
    network: BitcoinNetwork,
) -> InscribeResult<Brc20TransferTransactions> {
    let leftovers_address = get_address(leftovers_address, network)?;
    let transfer_dst_address = get_address(dst_address, network)?;

    let wallet = CanisterWallet::new(network, signer.clone());
    let inscription_dst_address = wallet.get_bitcoin_address(&H160::default()).await;
    let inscription_leftovers_address = inscription_dst_address.clone();

    let inscribe_txs = inscribe(
        Protocol::Brc20,
        inscription,
        inscription_dst_address.to_string(),
        inscription_leftovers_address.to_string(),
        multisig_config,
        signer,
        network,
    )
    .await?;

    let (transfer_tx, leftover_amount) = wallet
        .transfer_utxo(
            Txid::from_str(&inscribe_txs.commit_tx).unwrap(),
            Txid::from_str(&inscribe_txs.reveal_tx).unwrap(),
            transfer_dst_address,
            leftovers_address,
            inscribe_txs.leftover_amount,
        )
        .await?;

    Ok(Brc20TransferTransactions {
        commit_tx: inscribe_txs.commit_tx,
        reveal_tx: inscribe_txs.reveal_tx,
        transfer_tx: transfer_tx.to_string(),
        leftover_amount,
    })
}

/// Gets the Bitcoin address for the given derivation path.
pub async fn get_bitcoin_address(
    eth_address: H160,
    network: BitcoinNetwork,
    signer: EcdsaSigner,
) -> String {
    CanisterWallet::new(network, signer)
        .get_bitcoin_address(&eth_address)
        .await
        .to_string()
}

pub async fn get_inscription_fees(
    inscription_type: Protocol,
    inscription: String,
    multisig_config: Option<Multisig>,
    network: BitcoinNetwork,
    signer: EcdsaSigner,
) -> InscribeResult<InscriptionFees> {
    let multisig_config = multisig_config.map(|m| MultisigConfig {
        required: m.required,
        total: m.total,
    });

    CanisterWallet::new(network, signer)
        .get_inscription_fees(inscription_type, inscription, multisig_config)
        .await
}

/// Returns the parsed address given the string representation and the expected network.
#[inline]
pub(crate) fn get_address(address: String, network: BitcoinNetwork) -> InscribeResult<Address> {
    Address::from_str(&address)
        .map_err(|_| InscribeError::BadAddress(address.clone()))?
        .require_network(CanisterWallet::map_network(network))
        .map_err(|_| InscribeError::BadAddress(address))
}
