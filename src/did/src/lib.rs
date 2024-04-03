mod build_data;

use candid::CandidType;
use ord_rs::OrdError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use self::build_data::BuildData;

pub type InscribeResult<T> = Result<T, InscribeError>;

#[derive(Debug, Clone, CandidType, Serialize, Deserialize)]
/// The InscribeTransactions struct is used to return the commit and reveal transactions.
pub struct InscribeTransactions {
    pub commit_tx: String,
    pub reveal_tx: String,
}

/// Error type for inscribe endpoint.
#[derive(Debug, Clone, CandidType, Error)]
pub enum InscribeError {
    #[error("bad address: {0}")]
    BadAddress(String),
    #[error("bad inscription: {0}")]
    BadInscription(String),
    #[error("inscribe error: {0}")]
    OrdError(String),
    #[error("failed to collect utxos: {0}")]
    FailedToCollectUtxos(String),
    #[error("signature error {0}")]
    SignatureError(String),
}

impl From<OrdError> for InscribeError {
    fn from(e: OrdError) -> Self {
        InscribeError::OrdError(e.to_string())
    }
}

impl From<ethers_core::types::SignatureError> for InscribeError {
    fn from(e: ethers_core::types::SignatureError) -> Self {
        InscribeError::SignatureError(e.to_string())
    }
}

impl From<jsonrpc_core::Error> for InscribeError {
    fn from(e: jsonrpc_core::Error) -> Self {
        InscribeError::OrdError(e.to_string())
    }
}

#[derive(Debug, Clone, CandidType)]
pub struct InscriptionFees {
    pub commit_fee: u64,
    pub reveal_fee: u64,
    pub postage: u64,
}


