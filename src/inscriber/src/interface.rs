pub mod bitcoin_api;
pub mod ecdsa_api;

use candid::CandidType;
use ord_rs::{Inscription, OrdError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize)]
pub enum InscriptionWrapper {
    Brc20(ord_rs::Brc20),
    Nft(ord_rs::Nft),
}

#[derive(Debug, Clone, CandidType, Serialize, Deserialize, Default)]
pub struct InscriptionFees {
    pub commit_fee: u64,
    pub reveal_fee: u64,
    pub transfer_fee: Option<u64>,
    pub postage: u64,
    pub leftover_amount: u64,
}

/// Type of digital artifact being inscribed.
#[derive(CandidType, Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Protocol {
    Brc20,
    Nft,
}

/// BRC-20 operations
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub enum Brc20 {
    /// Deploy a BRC-20 token
    Deploy(Brc20Deploy),
    /// Mint BRC-20 tokens
    Mint(Brc20Mint),
    /// Transfer BRC-20 tokens
    Transfer(Brc20Transfer),
}

/// BRC-20 deploy operation
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct Brc20Deploy {
    pub tick: String,
    pub max: u64,
    pub lim: Option<u64>,
    pub dec: Option<u64>,
}

/// BRC-20 mint operation
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct Brc20Mint {
    pub tick: String,
    pub amt: u64,
}

/// BRC-20 transfer operation
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct Brc20Transfer {
    pub tick: String,
    pub amt: u64,
}

#[derive(CandidType, Clone, Debug, Serialize, Default, Deserialize)]
pub struct Nft {
    /// The MIME type of the body. This describes
    /// the format of the body content, such as "image/png" or "text/plain".
    pub content_type: String,
    /// The main body of the NFT. This is the core data or content of the NFT,
    /// which might represent an image, text, or other types of digital assets.
    pub body: String,
}

/// Represents multisig configuration (m of n) for a transaction, if applicable.
/// Encapsulates the number of required signatures and the total number of signatories.
#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
pub struct Multisig {
    /// Number of required signatures (m)
    pub required: usize,
    /// Total number of signatories (n)
    pub total: usize,
}

pub type InscribeResult<T> = Result<T, InscribeError>;

/// The InscribeTransactions struct is used to return the commit and reveal transactions.
#[derive(Debug, Clone, CandidType, Serialize, Deserialize)]
pub struct InscribeTransactions {
    pub commit_tx: String,
    pub reveal_tx: String,
    pub leftover_amount: u64,
}

/// The InscribeTransactions struct is used to return the commit and reveal transactions.
#[derive(Debug, Clone, CandidType, Serialize, Deserialize)]
pub struct Brc20TransferTransactions {
    pub commit_tx: String,
    pub reveal_tx: String,
    pub transfer_tx: String,
    pub leftover_amount: u64,
}

/// Error type for inscribe endpoint.
#[derive(Debug, Clone, CandidType, Error, Deserialize)]
pub enum InscribeError {
    #[error("bad address: {0}")]
    BadAddress(String),
    #[error("bad inscription: {0}")]
    BadInscription(String),
    #[error("inscribe error: {0}")]
    OrdError(String),
    #[error("failed to collect utxos: {0}")]
    FailedToCollectUtxos(String),
    #[error("no such utxo: {0}")]
    NoSuchUtxo(String),
    #[error("not enough UTXOs allocated for fees: {0}")]
    InsufficientFundsForFees(String),
    #[error("not enough UTXOs allocated for inscriptions: {0}")]
    InsufficientFundsForInscriptions(String),
    #[error("signature error {0}")]
    SignatureError(String),
    #[error("request error: {0}")]
    RequestError(String),
    #[error("address mismatch expected: {expected} actual: {actual}")]
    AddressMismatch { expected: String, actual: String },
    #[error("{0}")]
    DerivationPath(#[from] GetAddressError),
}

#[derive(Debug, Clone, Copy, CandidType, Deserialize, PartialEq, Eq, Error)]
pub enum GetAddressError {
    #[error("")]
    Derivation,
}

impl From<ord_rs::Brc20> for InscriptionWrapper {
    fn from(inscription: ord_rs::Brc20) -> Self {
        Self::Brc20(inscription)
    }
}

impl From<ord_rs::Nft> for InscriptionWrapper {
    fn from(inscription: ord_rs::Nft) -> Self {
        Self::Nft(inscription)
    }
}

impl Inscription for InscriptionWrapper {
    fn content_type(&self) -> String {
        match self {
            Self::Brc20(inscription) => inscription.content_type(),
            Self::Nft(inscription) => Inscription::content_type(inscription),
        }
    }

    fn data(&self) -> ord_rs::OrdResult<bitcoin::script::PushBytesBuf> {
        match self {
            Self::Brc20(inscription) => inscription.data(),
            Self::Nft(inscription) => inscription.data(),
        }
    }

    fn encode(&self) -> ord_rs::OrdResult<String>
    where
        Self: Serialize,
    {
        match self {
            Self::Brc20(inscription) => inscription.encode(),
            Self::Nft(inscription) => inscription.encode(),
        }
    }

    fn generate_redeem_script(
        &self,
        builder: bitcoin::script::Builder,
        pubkey: ord_rs::wallet::RedeemScriptPubkey,
    ) -> ord_rs::OrdResult<bitcoin::script::Builder> {
        match self {
            Self::Brc20(inscription) => inscription.generate_redeem_script(builder, pubkey),
            Self::Nft(inscription) => inscription.generate_redeem_script(builder, pubkey),
        }
    }

    fn parse(_data: &[u8]) -> ord_rs::OrdResult<Self>
    where
        Self: Sized,
    {
        unimplemented!()
    }
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
        InscribeError::RequestError(e.to_string())
    }
}
