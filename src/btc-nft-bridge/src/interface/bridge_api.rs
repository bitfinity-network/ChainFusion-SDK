use candid::CandidType;
use did::{H160, H256};
use inscriber::interface::{Multisig, Protocol};
use minter_did::erc721_mint_order::ERC721SignedMintOrder;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::store::StorableInscriptionId;

#[derive(Error, CandidType, Clone, Debug, Deserialize, PartialEq, Eq)]
pub enum BridgeError {
    #[error("inscription parsing error: {0}")]
    InscriptionParsing(String),
    #[error("malformed address: {0}")]
    MalformedAddress(String),
    #[error("fetch nft token details error: {0}")]
    FetchNftTokenDetails(String),
    #[error("get transaction error: {0}")]
    GetTransactionById(String),
    #[error("get utxos error: {0}")]
    GetUtxos(String),
    #[error("no such utxo error: {0}")]
    NoSuchUtxo(String),
    #[error("public key from string error: {0}")]
    PublicKeyFromStr(String),
    #[error("address from public key error: {0}")]
    AddressFromPublicKey(String),
    #[error("ecdsa public key error: {0}")]
    EcdsaPublicKey(String),
    #[error("signature error: {0}")]
    SignatureError(String),
    #[error("set token symbol error: {0}")]
    SetTokenSymbol(String),
    #[error("find inscription utxo error: {0}")]
    FindInscriptionUtxo(String),
    #[error("erc721 mint error: {0}")]
    Erc721Mint(#[from] NftMintError),
    #[error("erc721 burn error: {0}")]
    Erc721Burn(String),
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub enum DepositError {
    Pending {
        min_confirmations: u32,
        current_confirmations: u32,
    },
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct InscribeNftArgs {
    pub inscription_type: Protocol,
    pub inscription: String,
    pub leftovers_address: String,
    pub dst_address: String,
    pub multisig_config: Option<Multisig>,
}

/// Arguments to `NftTask::MintNft`
#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MintNftArgs {
    /// User's ETH address
    pub eth_address: H160,
    /// User's BTC address
    pub btc_address: String,
    /// NFT id
    pub nft_id: StorableInscriptionId,
}

/// Status of an NFT to a BTC NFT swap
#[derive(CandidType, Clone, Debug, Deserialize)]
pub struct NftInscribeStatus {
    pub tx_id: String,
}

/// Errors that occur during an NFT to a BTC NFT swap.
#[derive(Error, CandidType, Clone, Debug, Deserialize)]
pub enum NftInscribeError {
    /// Error returned by the `inscribe` endpoint of the Inscriber.
    #[error("{0}")]
    Inscribe(String),
    /// There are too many concurrent requests, retry later.
    #[error("{0}")]
    TemporarilyUnavailable(String),
}

/// Status of a NFT to BTC-NFT swap
#[derive(Debug, CandidType, Deserialize, PartialEq, Eq)]
pub enum NftMintStatus {
    /// This happens when the transaction is processed, the BRC20 inscription is parsed and validated,
    /// and the mint order is created; however, there is a problem sending the mint order to the EVM.
    /// The signed mint order can be sent manually to the BftBridge to mint wrapped tokens.
    Signed(Box<ERC721SignedMintOrder>),
    /// Mint order for wrapped tokens is successfully sent to the `BftBridge`.
    Minted {
        /// Id of the minted NFT
        id: StorableInscriptionId,
        /// EVM transaction ID.
        tx_id: H256,
    },
}

/// Errors that occur during a BTC-NFT to NFT swap.
#[derive(Error, Debug, Clone, CandidType, Deserialize, PartialEq, Eq)]
pub enum NftMintError {
    /// Error from the NftBridge
    #[error("{0}")]
    NftBridge(String),
    /// The NftBridge is not properly initialized.
    #[error("{0}")]
    NotInitialized(String),
    /// Error connecting to the EVM.
    #[error("{0}")]
    Evm(String),
    /// The inscription (NFT) received is invalid.
    #[error("{0}")]
    InvalidNft(String),
    /// Error while signing the mint order.
    #[error("{0}")]
    Sign(String),
}
