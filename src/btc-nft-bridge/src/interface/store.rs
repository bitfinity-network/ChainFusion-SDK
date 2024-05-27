use std::borrow::Cow;
use std::rc::Rc;

use bitcoin::{OutPoint, Txid};
use candid::types::{Type, TypeInner};
use candid::{CandidType, Decode, Deserialize, Encode};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{BTreeMapStructure, Bound, StableBTreeMap, Storable, VirtualMemory};
use minter_contract_utils::erc721_mint_orders::MintOrders;
use minter_did::erc721_mint_order::ERC721SignedMintOrder;
use minter_did::id256::Id256;
use ord_rs::inscription::iid::InscriptionId;
use serde::Serialize;

use super::bridge_api::BridgeError;
use crate::memory::{
    BURN_REQUEST_MEMORY_ID, MEMORY_MANAGER, MINT_ORDERS_MEMORY_ID, NFT_STORE_MEMORY_ID,
};

const SRC_TOKEN: Id256 = Id256([0; 32]);

pub type RevealTxId = String;

pub struct NftStore {
    inner: StableBTreeMap<RevealTxId, NftInfo, VirtualMemory<DefaultMemoryImpl>>,
}

impl Default for NftStore {
    fn default() -> Self {
        Self {
            inner: StableBTreeMap::new(MEMORY_MANAGER.with(|mm| mm.get(NFT_STORE_MEMORY_ID))),
        }
    }
}

impl NftStore {
    pub fn get_nft_info(&self, txid: &str) -> Option<NftInfo> {
        self.inner.get(&txid.to_string())
    }

    pub fn insert(&mut self, token_info: NftInfo) {
        self.inner
            .insert(token_info.id.0.txid.to_string(), token_info.clone());
    }

    pub fn remove(&mut self, txid: String) -> Result<(), String> {
        match self.inner.remove(&txid) {
            Some(_v) => Ok(()),
            None => Err("Token not found in store".to_string()),
        }
    }

    pub(crate) fn has_inscription(&self, txid: &str) -> bool {
        self.get_nft_info(txid).is_some()
    }
}

#[derive(Debug, CandidType, Deserialize, Clone, Eq, PartialEq)]
pub struct NftInfo {
    vout: u32,
    pub id: StorableInscriptionId,
    pub holder: String,
}

impl NftInfo {
    pub fn new(
        id: StorableInscriptionId,
        holder: String,
        output: String,
    ) -> Result<Self, BridgeError> {
        let output = output.split(':');
        let vout = output
            .clone()
            .nth(1)
            .unwrap()
            .parse::<u32>()
            .map_err(|e| BridgeError::MalformedAddress(e.to_string()))?;

        Ok(Self { id, holder, vout })
    }
}

impl From<&NftInfo> for OutPoint {
    fn from(value: &NftInfo) -> Self {
        OutPoint {
            txid: value.id.0.txid,
            vout: value.vout,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Eq, PartialEq)]
pub struct StorableInscriptionId(pub InscriptionId);

impl CandidType for StorableInscriptionId {
    fn _ty() -> Type {
        Type(Rc::new(TypeInner::Text))
    }

    fn idl_serialize<S>(&self, serializer: S) -> Result<(), S::Error>
    where
        S: candid::types::Serializer,
    {
        serializer.serialize_text(&self.0.to_string())
    }
}

impl From<InscriptionId> for StorableInscriptionId {
    fn from(nft_id: InscriptionId) -> Self {
        Self(nft_id)
    }
}

impl From<StorableInscriptionId> for InscriptionId {
    fn from(storable_nft_id: StorableInscriptionId) -> Self {
        storable_nft_id.0
    }
}

#[derive(Debug, Deserialize, Clone, Eq, PartialEq)]
pub struct StorableTxId(pub Txid);

impl CandidType for StorableTxId {
    fn _ty() -> Type {
        Type(Rc::new(TypeInner::Text))
    }

    fn idl_serialize<S>(&self, serializer: S) -> Result<(), S::Error>
    where
        S: candid::types::Serializer,
    {
        serializer.serialize_text(&self.0.to_string())
    }
}

impl Storable for NftInfo {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(&(self,)).expect("serialization failed"))
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, (Self,)).expect("deserialization failed").0
    }

    const BOUND: Bound = Bound::Unbounded;
}

pub struct MintOrdersStore(MintOrders<VirtualMemory<DefaultMemoryImpl>>);

impl Default for MintOrdersStore {
    fn default() -> Self {
        Self(MintOrders::new(
            MEMORY_MANAGER.with(|mm| mm.get(MINT_ORDERS_MEMORY_ID)),
        ))
    }
}

impl MintOrdersStore {
    pub fn push(&mut self, sender: Id256, nonce: u32, mint_order: ERC721SignedMintOrder) {
        self.0.insert(sender, SRC_TOKEN, nonce, &mint_order);
    }

    pub fn remove(&mut self, sender: Id256, nonce: u32) {
        self.0.remove(sender, SRC_TOKEN, nonce);
    }
}

pub type BurnRequestId = u32;

pub struct BurnRequestStore {
    inner: StableBTreeMap<BurnRequestId, BurnRequestInfo, VirtualMemory<DefaultMemoryImpl>>,
}

impl Default for BurnRequestStore {
    fn default() -> Self {
        Self {
            inner: StableBTreeMap::new(MEMORY_MANAGER.with(|mm| mm.get(BURN_REQUEST_MEMORY_ID))),
        }
    }
}

impl BurnRequestStore {
    pub fn remove(&mut self, request_id: BurnRequestId) {
        self.inner.remove(&request_id);
    }

    pub fn set_transferred(&mut self, request_id: BurnRequestId) {
        if let Some(v) = self.inner.remove(&request_id) {
            self.inner.insert(
                request_id,
                BurnRequestInfo {
                    is_transferred: true,
                    ..v
                },
            );
        }
    }
}

#[derive(Debug, Clone, CandidType, Deserialize)]
struct BurnRequestInfo {
    address: String,
    reveal_txid: RevealTxId,
    is_transferred: bool,
}

impl Storable for BurnRequestInfo {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(&(self,)).expect("serialization failed"))
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, (Self,)).expect("deserialization failed").0
    }

    const BOUND: Bound = Bound::Unbounded;
}

#[cfg(test)]
mod test {

    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_should_encode_nftid() {
        let txid =
            Txid::from_str("2ca04a8c189d1eabdad4dafb654cd2ead33a17be983cf77103e585158c957262")
                .unwrap();

        let nft_id = InscriptionId {
            txid: txid.clone(),
            index: 1,
        };

        let storable_nft_id = StorableInscriptionId(nft_id);
        let bytes = Encode!(&(storable_nft_id.clone(),)).expect("serialization failed");
        let decoded = Decode!(&bytes, (StorableInscriptionId,))
            .expect("deserialization failed")
            .0;

        assert_eq!(storable_nft_id, decoded);
    }
}
