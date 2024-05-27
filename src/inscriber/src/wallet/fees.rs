use bitcoin::absolute::LockTime;
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, FeeRate, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use ord_rs::wallet::ScriptType;
use ord_rs::MultisigConfig;

/// Single ECDSA signature + SIGHASH type size in bytes.
const ECDSA_SIGHASH_SIZE: usize = 72 + 1;
/// Single Schnorr signature + SIGHASH type size for Taproot in bytes.
const SCHNORR_SIGHASH_SIZE: usize = 64 + 1;

pub fn inscription_tranfer_fees(fee_rate: &FeeRate, recipient_address: &Address) -> Amount {
    let tx_out = vec![
        TxOut {
            value: Amount::ONE_SAT,
            script_pubkey: recipient_address.script_pubkey(),
        };
        2
    ];
    estimate_transaction_fees(ScriptType::P2WSH, 2, fee_rate, &None, tx_out)
}

pub fn estimate_transaction_fees(
    script_type: ScriptType,
    number_of_inputs: usize,
    current_fee_rate: &FeeRate,
    multisig_config: &Option<MultisigConfig>,
    outputs: Vec<TxOut>,
) -> Amount {
    let vbytes = estimate_vbytes(number_of_inputs, script_type, multisig_config, outputs);

    current_fee_rate.fee_vb(vbytes as u64).unwrap()
}

fn estimate_vbytes(
    inputs: usize,
    script_type: ScriptType,
    multisig_config: &Option<MultisigConfig>,
    outputs: Vec<TxOut>,
) -> usize {
    let sighash_size = match script_type {
        // For P2WSH, calculate based on the multisig configuration if provided.
        ScriptType::P2WSH => match multisig_config {
            Some(config) => ECDSA_SIGHASH_SIZE * config.required,
            None => ECDSA_SIGHASH_SIZE, // Default to single signature size if no multisig config is provided.
        },
        // For P2TR, use the fixed Schnorr signature size.
        ScriptType::P2TR => SCHNORR_SIGHASH_SIZE,
    };

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: (0..inputs)
            .map(|_| TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::from_slice(&[&vec![0; sighash_size]]),
            })
            .collect(),
        output: outputs,
    }
    .vsize()
}
