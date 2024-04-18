mod fees;
mod signer;

use std::cell::RefCell;
use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::consensus::serialize;
use bitcoin::hashes::Hash;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, FeeRate, Network, OutPoint, PublicKey, ScriptBuf, Sequence, Transaction, TxIn,
    TxOut, Txid, Witness,
};
use hex::ToHex;
use ic_exports::ic_cdk::api::management_canister::bitcoin::{BitcoinNetwork, Outpoint, Utxo};
use ord_rs::constants::POSTAGE;
use ord_rs::wallet::ScriptType;
use ord_rs::{
    Brc20, CreateCommitTransaction, CreateCommitTransactionArgs, ExternalSigner, Inscription,
    MultisigConfig, Nft, OrdError, OrdResult, OrdTransactionBuilder, RevealTransactionArgs,
    SignCommitTransactionArgs, Utxo as OrdUtxo, Wallet, WalletType,
};
use serde::de::DeserializeOwned;

use self::signer::{Signer, Spender};
use crate::interface::inscriber_api::{
    InscribeError, InscribeResult, InscribeTransactions, InscriptionFees, InscriptionWrapper,
    Nft as CandidNft, Protocol,
};
use crate::interface::{bitcoin_api, ecdsa_api};
use crate::state::State;

#[derive(Clone)]
pub struct EcdsaSigner {
    derivation_path: Vec<Vec<u8>>,
}

#[async_trait::async_trait]
impl ExternalSigner for EcdsaSigner {
    async fn ecdsa_public_key(&self) -> String {
        match ecdsa_api::ecdsa_public_key(self.derivation_path.clone()).await {
            Ok(res) => res.public_key_hex,
            Err(e) => panic!("{e}"),
        }
    }

    async fn sign_with_ecdsa(&self, message: &str) -> String {
        match ecdsa_api::sign_with_ecdsa(self.derivation_path.clone(), message).await {
            Ok(res) => res.signature_hex,
            Err(e) => panic!("{e}"),
        }
    }

    async fn verify_ecdsa(&self, signature_hex: &str, message: &str, public_key_hex: &str) -> bool {
        match ecdsa_api::verify_ecdsa(signature_hex, message, public_key_hex).await {
            Ok(res) => res.is_signature_valid,
            Err(e) => panic!("{e}"),
        }
    }
}

pub struct CanisterWallet {
    bitcoin_network: BitcoinNetwork,
    derivation_path: Vec<Vec<u8>>,
    network: Network,
}

impl CanisterWallet {
    pub fn new(derivation_path: Vec<Vec<u8>>, network: BitcoinNetwork) -> Self {
        Self {
            bitcoin_network: network,
            derivation_path,
            network: Self::map_network(network),
        }
    }

    #[inline]
    fn with_ecdsa_signer(signer: EcdsaSigner) -> Wallet {
        Wallet::new_with_signer(WalletType::External {
            signer: Box::new(signer),
        })
    }

    #[inline]
    fn ecdsa_signer(&self) -> EcdsaSigner {
        EcdsaSigner {
            derivation_path: self.derivation_path.clone(),
        }
    }

    /// Returns bech32 bitcoin `Address` of this canister.
    pub async fn get_bitcoin_address(&self) -> Address {
        let public_key = match ecdsa_api::ecdsa_public_key(self.derivation_path.clone()).await {
            Ok(res) => res.public_key_hex,
            Err(e) => panic!("{e}"),
        };

        let pk = PublicKey::from_str(&public_key).expect("Can't deserialize public key");
        Self::btc_address_from_public_key(self.bitcoin_network, &pk)
    }

    /// Returns the estimated inscription fees for the given inscription.
    pub async fn get_inscription_fees(
        &self,
        inscription_type: Protocol,
        inscription: String,
        multisig_config: Option<MultisigConfig>,
    ) -> InscribeResult<InscriptionFees> {
        use crate::constant::{DUMMY_BITCOIN_ADDRESS, DUMMY_BITCOIN_PUBKEY};

        let ecdsa_signer = self.ecdsa_signer();
        let own_utxos = vec![OrdUtxo {
            id: Txid::all_zeros(),
            index: 0,
            amount: Amount::MAX,
        }];
        let dummy_pubkey = PublicKey::from_str(DUMMY_BITCOIN_PUBKEY).unwrap();
        let dummy_address = Address::from_str(DUMMY_BITCOIN_ADDRESS)
            .unwrap()
            .assume_checked();

        // initialize a wallet (transaction signer) and a transaction builder
        let wallet = Self::with_ecdsa_signer(ecdsa_signer);
        // Hardcoded for debugging
        let script_type = ScriptType::P2WSH;
        let mut builder = OrdTransactionBuilder::new(dummy_pubkey, script_type, wallet);

        let dst_address = dummy_address.clone();
        let leftovers_address = dummy_address.clone();
        let fee_rate = self.get_fee_rate().await;

        let inscription = self.build_inscription(inscription_type, inscription)?;
        let transfer_fee = if matches!(inscription, InscriptionWrapper::Brc20(Brc20::Transfer(_))) {
            Some(fees::inscription_tranfer_fees(&fee_rate, &dst_address))
        } else {
            None
        };
        let commit_tx_result = self.build_commit_transaction(
            &mut builder,
            inscription,
            dst_address.clone(),
            leftovers_address,
            dummy_address,
            &own_utxos,
            fee_rate,
            multisig_config,
        )?;

        Ok(InscriptionFees {
            commit_fee: commit_tx_result.commit_fee.to_sat(),
            reveal_fee: commit_tx_result.reveal_fee.to_sat(),
            transfer_fee: transfer_fee.map(|amt| amt.to_sat()),
            postage: POSTAGE,
            leftover_amount: commit_tx_result.leftover_amount.to_sat(),
        })
    }

    /// Handles the inscription flow.
    ///
    /// Returns the transaction IDs for both the commit and reveal transactions.
    pub async fn inscribe(
        &self,
        state: &RefCell<State>,
        inscription_type: Protocol,
        inscription: String,
        dst_address: Address,
        leftovers_address: Address,
        multisig_config: Option<MultisigConfig>,
    ) -> InscribeResult<InscribeTransactions> {
        let ecdsa_signer = self.ecdsa_signer();

        let own_pk = PublicKey::from_str(&ecdsa_signer.ecdsa_public_key().await)
            .map_err(OrdError::PubkeyConversion)?;

        let own_address = Self::btc_address_from_public_key(self.bitcoin_network, &own_pk);

        log::info!("Fetching UTXOs...");
        let own_utxos = bitcoin_api::get_utxos(self.bitcoin_network, own_address.to_string())
            .await
            .map_err(InscribeError::FailedToCollectUtxos)?
            .utxos;

        log::info!("Getting inscription fees...");
        let fees = self
            .get_inscription_fees(
                inscription_type,
                inscription.clone(),
                multisig_config.clone(),
            )
            .await?;

        log::info!("Processing UTXOs...");
        let own_utxos = state.borrow_mut().process_utxos(fees, own_utxos)?;

        // initialize a wallet (transaction signer) and a transaction builder
        let wallet = Self::with_ecdsa_signer(ecdsa_signer);
        // Hardcoded for debugging
        // TODO: dynamically determine the `ScriptType`
        let script_type = ScriptType::P2WSH;
        let mut builder = OrdTransactionBuilder::new(own_pk, script_type, wallet);

        let fee_rate = self.get_fee_rate().await;

        log::info!("Building commit transaction inputs...");
        let commit_tx_inputs = self.build_commit_transaction_inputs(&own_utxos);

        log::info!("Parsing the inscription...");
        let inscription = self.build_inscription(inscription_type, inscription)?;

        log::info!("Building the commit transaction...");
        let commit_tx_result = self.build_commit_transaction(
            &mut builder,
            inscription,
            dst_address.clone(),
            leftovers_address,
            own_address.clone(),
            &commit_tx_inputs,
            fee_rate,
            multisig_config,
        )?;

        log::info!("Signing the commit transaction...");
        let commit_tx = Self::sign_commit_transaction(
            &mut builder,
            commit_tx_result.unsigned_tx,
            SignCommitTransactionArgs {
                inputs: commit_tx_inputs,
                txin_script_pubkey: own_address.script_pubkey(),
            },
        )
        .await?;

        log::info!("Building and signing the reveal transaction...");
        let reveal_tx = Self::build_reveal_transaction(
            &mut builder,
            &commit_tx,
            commit_tx_result.reveal_balance,
            commit_tx_result.redeem_script,
            dst_address,
        )
        .await?;

        log::info!("Sending the commit transaction...");
        bitcoin_api::send_transaction(self.bitcoin_network, serialize(&commit_tx)).await;
        log::info!("Done");

        log::info!("Sending the reveal transaction...");
        bitcoin_api::send_transaction(self.bitcoin_network, serialize(&reveal_tx)).await;
        log::info!("Done");

        // Clear the locked UTXO set
        state.borrow_mut().reset_utxo_vault();

        Ok(InscribeTransactions {
            commit_tx: commit_tx.txid().encode_hex(),
            reveal_tx: reveal_tx.txid().encode_hex(),
            leftover_amount: commit_tx_result.leftover_amount.to_sat(),
        })
    }

    /// Transfer a UTXO from the canister to a recipient address.
    pub async fn transfer_utxo(
        &self,
        commit_txid: Txid,
        reveal_txid: Txid,
        recipient_address: Address,
        leftovers_address: Address,
        leftover_amount: u64,
    ) -> InscribeResult<(Txid, u64)> {
        let own_address = self.get_bitcoin_address().await;

        let fee_rate = self.get_fee_rate().await;
        let fee_utxo = Utxo {
            outpoint: Outpoint {
                txid: commit_txid.as_byte_array().to_vec(),
                vout: 1,
            },
            value: leftover_amount,
            height: 0,
        };
        let inscription_utxo = Utxo {
            outpoint: Outpoint {
                txid: reveal_txid.as_byte_array().to_vec(),
                vout: 0,
            },
            value: POSTAGE,
            height: 0,
        };

        let transfer_fee = fees::inscription_tranfer_fees(&fee_rate, &recipient_address);
        let leftover_amount = Amount::from_sat(fee_utxo.value) - transfer_fee;

        // build transaction
        let input = [&inscription_utxo, &fee_utxo]
            .map(|utxo| TxIn {
                previous_output: OutPoint {
                    txid: Txid::from_slice(&utxo.outpoint.txid).expect("Failed to parse txid"),
                    vout: utxo.outpoint.vout,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::from_consensus(0xffffffff),
                witness: Witness::new(),
            })
            .to_vec();

        let output = vec![
            TxOut {
                value: Amount::from_sat(inscription_utxo.value),
                script_pubkey: recipient_address.script_pubkey(),
            },
            TxOut {
                value: leftover_amount,
                script_pubkey: leftovers_address.script_pubkey(),
            },
        ];

        let unsigned_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input,
            output,
        };

        // sign transaction
        let ecdsa_signer = EcdsaSigner {
            derivation_path: self.derivation_path.clone(),
        };

        let own_pk = PublicKey::from_str(&ecdsa_signer.ecdsa_public_key().await)
            .map_err(OrdError::PubkeyConversion)?;
        let signer = Signer::new(ecdsa_signer);

        let utxos_to_sign = [inscription_utxo, fee_utxo]
            .map(|utxo| OrdUtxo {
                id: Txid::from_slice(&utxo.outpoint.txid).expect("Failed to parse txid"),
                index: utxo.outpoint.vout,
                amount: Amount::from_sat(utxo.value),
            })
            .to_vec();

        let spender = Spender {
            pubkey: own_pk,
            script: own_address.script_pubkey(),
        };
        let signed_tx = signer
            .sign_transaction_ecdsa(unsigned_tx, &utxos_to_sign, spender)
            .await?;

        // send transaction
        bitcoin_api::send_transaction(self.bitcoin_network, serialize(&signed_tx)).await;

        Ok((signed_tx.txid(), leftover_amount.to_sat()))
    }

    fn build_inscription(
        &self,
        inscription_type: Protocol,
        inscription: String,
    ) -> InscribeResult<InscriptionWrapper> {
        match inscription_type {
            Protocol::Brc20 => {
                let op: Brc20 = serde_json::from_str(&inscription)
                    .map_err(|e| InscribeError::BadInscription(e.to_string()))?;

                let inscription = match op {
                    Brc20::Deploy(data) => Brc20::deploy(data.tick, data.max, data.lim, data.dec),
                    Brc20::Mint(data) => Brc20::mint(data.tick, data.amt),
                    Brc20::Transfer(data) => Brc20::transfer(data.tick, data.amt),
                };

                Ok(inscription.into())
            }
            Protocol::Nft => {
                let data: CandidNft = serde_json::from_str(&inscription)
                    .map_err(|e| InscribeError::BadInscription(e.to_string()))?;
                let inscription = Nft::new(
                    Some(data.content_type.as_bytes().to_vec()),
                    Some(data.body.as_bytes().to_vec()),
                );

                Ok(inscription.into())
            }
        }
    }

    fn build_commit_transaction_inputs(&self, own_utxos: &[Utxo]) -> Vec<OrdUtxo> {
        own_utxos
            .iter()
            .map(|utxo| OrdUtxo {
                id: Txid::from_raw_hash(
                    Hash::from_slice(&utxo.outpoint.txid).expect("Failed to parse txid"),
                ),
                index: utxo.outpoint.vout,
                amount: Amount::from_sat(utxo.value),
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn build_commit_transaction<T>(
        &self,
        builder: &mut OrdTransactionBuilder,
        inscription: T,
        recipient_address: Address,
        leftovers_address: Address,
        own_address: Address,
        tx_inputs: &[OrdUtxo],
        fee_rate: FeeRate,
        multisig_config: Option<MultisigConfig>,
    ) -> OrdResult<CreateCommitTransaction>
    where
        T: Inscription + DeserializeOwned,
    {
        let commit_tx_args = CreateCommitTransactionArgs {
            inputs: tx_inputs.to_vec(),
            inscription,
            leftovers_recipient: leftovers_address,
            txin_script_pubkey: own_address.script_pubkey(),
            fee_rate,
            multisig_config,
        };

        let commit_tx_result = builder.build_commit_transaction(
            self.network,
            recipient_address.clone(),
            commit_tx_args,
        )?;

        Ok(commit_tx_result)
    }

    async fn sign_commit_transaction(
        builder: &mut OrdTransactionBuilder,
        unsigned_tx: Transaction,
        sign_args: SignCommitTransactionArgs,
    ) -> OrdResult<Transaction> {
        builder
            .sign_commit_transaction(unsigned_tx, sign_args)
            .await
    }

    async fn build_reveal_transaction(
        builder: &mut OrdTransactionBuilder,
        commit_tx: &Transaction,
        reveal_balance: Amount,
        redeem_script: ScriptBuf,
        recipient_address: Address,
    ) -> OrdResult<Transaction> {
        let reveal_tx_args = RevealTransactionArgs {
            input: OrdUtxo {
                id: commit_tx.txid(),
                index: 0,
                amount: reveal_balance,
            },
            recipient_address,
            redeem_script,
        };

        builder.build_reveal_transaction(reveal_tx_args).await
    }

    // Returns bech32 bitcoin `Address` of this canister from given `PublicKey`.
    fn btc_address_from_public_key(network: BitcoinNetwork, pk: &PublicKey) -> Address {
        let network = Self::map_network(network);

        // Compute the bitcoin segwit(bech32) address.
        Address::p2wpkh(pk, network).expect("Can't convert public key to segwit bitcoin address")
    }

    async fn get_fee_rate(&self) -> FeeRate {
        // Get fee percentiles from previous transactions to estimate our own fee.
        let fee_percentiles = bitcoin_api::get_current_fee_percentiles(self.bitcoin_network).await;

        let fee_per_byte = if fee_percentiles.is_empty() {
            // There are no fee percentiles. This case can only happen on a regtest
            // network where there are no non-coinbase transactions. In this case,
            // we use a default of 2000 millisatoshis/byte (i.e. 2 satoshis/byte)
            2000
        } else {
            // Choose the 90th percentile for sending fees.
            fee_percentiles[90]
        };

        FeeRate::from_sat_per_vb(fee_per_byte).expect("Overflow!")
    }

    #[inline]
    pub fn map_network(network: BitcoinNetwork) -> Network {
        match network {
            BitcoinNetwork::Mainnet => Network::Bitcoin,
            BitcoinNetwork::Testnet => Network::Testnet,
            BitcoinNetwork::Regtest => Network::Regtest,
        }
    }
}
