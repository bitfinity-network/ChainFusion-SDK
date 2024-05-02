use std::borrow::Cow;

use candid::CandidType;
use did::H160;
use eth_signer::sign_strategy::TransactionSigner;
use ethers_core::types::Signature;
use ethers_core::utils::keccak256;
use ic_stable_structures::stable_structures::Memory;
use ic_stable_structures::{Bound, MultimapStructure as _, StableMultimap, Storable};
use minter_did::id256::Id256;
use serde::de::Visitor;
use serde::Deserialize;

#[derive(Debug, CandidType, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SignedMintOrder(pub Vec<u8>);

/// Visitor for `SignedMintOrder` objects deserialization.
struct SignedMintOrderVisitor;

impl<'v> Visitor<'v> for SignedMintOrderVisitor {
    type Value = SignedMintOrder;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "blob of size {}",
            MintOrder::SIGNED_ENCODED_DATA_SIZE
        )
    }

    fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(SignedMintOrder(
            v.try_into()
                .map_err(|_| E::invalid_length(v.len(), &Self))?,
        ))
    }
}

impl<'de> Deserialize<'de> for SignedMintOrder {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_bytes(SignedMintOrderVisitor)
    }
}

/// Data which should be signed and provided to the `BftBridge.mint()` call
/// to perform mint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintOrder {
    /// Identifier of the user who performs the mint.
    pub sender: Id256,

    /// Identifier of the source token.
    pub src_token: Id256,

    /// Address of the receiver of the mint.
    pub recipient: H160,

    /// Destination token for which mint operation will be performed.
    pub dst_token: H160,

    /// Value to prevent double spending.
    pub nonce: u32,

    /// ChainId of EVM on which user will send tokens to bridge.
    pub sender_chain_id: u32,

    /// ChainId of EVM on which will send tokens to user.
    /// Used to prevent several cross-chain mints with the same order.
    pub recipient_chain_id: u32,

    /// Name of the token.
    pub name: [u8; 32],

    /// Symbol of the token.
    pub symbol: [u8; 16],

    /// Mint operation should approve tokens, using this address as a spender.
    pub approve_spender: H160,

    /// Mint operation should approve this amount of tokens.
    pub token_uri: String,
}

impl MintOrder {
    pub const ENCODED_DATA_SIZE: usize = 188;
    pub const SIGNED_ENCODED_DATA_SIZE: usize = Self::ENCODED_DATA_SIZE + 65;

    /// Encodes order data and signs it.
    /// Encoded data layout:
    /// ```ignore
    /// [
    ///     0..32 bytes of sender,                 }
    ///     32..64 bytes of src_token,              }
    ///     64..84 bytes of recipient,             }
    ///     84..104 bytes of dst_token,            }
    ///     104..108 bytes of nonce,                } => signed data
    ///     108..112 bytes of sender_chain_id,      }
    ///     112..116 bytes of recipient_chain_id,   }
    ///     116..148 bytes of name,                 }
    ///     148..164 bytes of symbol,               }
    ///     164..184 bytes of spender,             }
    ///     184..188 bytes of data size,      }
    ///     188..188 + dataLen bytes of data,       }
    ///     188 + dataLen..188 + dataLen + 65 bytes of signature (r - 32 bytes, s - 32 bytes, v - 1 byte)
    /// ]
    /// ```
    ///
    /// All integers encoded in big-endian format.
    /// Signature signs KECCAK hash of the signed data.
    pub async fn encode_and_sign(
        &self,
        signer: &impl TransactionSigner,
    ) -> anyhow::Result<SignedMintOrder> {
        let data = self.token_uri.as_bytes();
        let mut buf = vec![0; Self::SIGNED_ENCODED_DATA_SIZE + data.len()];
        let data_size = data.len();
        let last_data_index = Self::ENCODED_DATA_SIZE + data_size;

        buf[0..32].copy_from_slice(self.sender.0.as_slice());
        buf[32..64].copy_from_slice(self.src_token.0.as_slice());
        buf[64..84].copy_from_slice(self.recipient.0.as_bytes());
        buf[84..104].copy_from_slice(self.dst_token.0.as_bytes());
        buf[104..108].copy_from_slice(&self.nonce.to_be_bytes());
        buf[108..112].copy_from_slice(&self.sender_chain_id.to_be_bytes());
        buf[112..116].copy_from_slice(&self.recipient_chain_id.to_be_bytes());
        buf[116..148].copy_from_slice(&self.name);
        buf[148..164].copy_from_slice(&self.symbol);
        buf[164..184].copy_from_slice(self.approve_spender.0.as_bytes());
        buf[184..188].copy_from_slice(&(data_size as u32).to_be_bytes());
        buf[188..last_data_index].copy_from_slice(&data);

        let digest = keccak256(&buf[..last_data_index]);

        // Sign fields data hash.
        let signature = signer
            .sign_digest(digest)
            .await
            .map_err(|e| anyhow::anyhow!("failed to sign MintOrder: {e}"))?;

        // Add signature to the data.
        let signature_bytes: [u8; 65] = ethers_core::types::Signature::from(signature).into();
        buf[last_data_index..].copy_from_slice(&signature_bytes);

        Ok(SignedMintOrder(buf))
    }

    /// Decode Self from bytes.
    pub fn decode_data(data: &[u8]) -> Option<Self> {
        if data.len() < Self::ENCODED_DATA_SIZE {
            return None;
        }

        let sender = data[0..32].try_into().unwrap(); // exactly 32 bytes, as expected
        let src_token = data[32..64].try_into().unwrap(); // exactly 32 bytes, as expected
        let recipient = H160::from_slice(&data[64..84]);
        let dst_token = H160::from_slice(&data[84..104]);
        let nonce = u32::from_be_bytes(data[104..108].try_into().unwrap()); // exactly 4 bytes, as expected
        let sender_chain_id = u32::from_be_bytes(data[108..112].try_into().unwrap()); // exactly 4 bytes, as expected
        let recipient_chain_id = u32::from_be_bytes(data[112..116].try_into().unwrap()); // exactly 4 bytes, as expected
        let name = data[116..148].try_into().unwrap(); // exactly 32 bytes, as expected
        let symbol = data[148..164].try_into().unwrap(); // exactly 16 bytes, as expected
        let approve_spender = H160::from_slice(&data[164..184]);
        let data_size = u32::from_be_bytes(data[184..188].try_into().unwrap()); // exactly 4 bytes, as expected
        let data = data[188..188 + data_size as usize].to_vec();
        let token_uri = String::from_utf8(data).unwrap();

        Some(Self {
            sender,
            src_token,
            recipient,
            dst_token,
            nonce,
            sender_chain_id,
            recipient_chain_id,
            name,
            symbol,
            approve_spender,
            token_uri,
        })
    }

    /// Decode Self from bytes.
    pub fn decode_signed(data: &SignedMintOrder) -> Option<(Self, Signature)> {
        if data.0.len() < Self::SIGNED_ENCODED_DATA_SIZE {
            return None;
        }

        let decoded_data = Self::decode_data(data.0.as_ref())?;
        let signature_start = Self::ENCODED_DATA_SIZE + decoded_data.token_uri.len();
        let signature =
            ethers_core::types::Signature::try_from(&data.0[signature_start..signature_start + 65])
                .ok()?
                .into();

        Some((decoded_data, signature))
    }
}

pub struct MintOrders<M: Memory> {
    mint_orders_map: StableMultimap<MintOrderKey, u32, SignedMintOrder, M>,
}

impl Storable for SignedMintOrder {
    const BOUND: Bound = Bound::Unbounded;

    fn to_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        self.0.to_bytes()
    }

    fn from_bytes(bytes: std::borrow::Cow<'_, [u8]>) -> Self {
        Self(<Vec<u8>>::from_bytes(bytes))
    }
}

impl<M: Memory> MintOrders<M> {
    pub fn new(memory: M) -> Self {
        Self {
            mint_orders_map: StableMultimap::new(memory),
        }
    }

    /// Inserts a new signed mint order.
    /// Returns replaced signed mint order if it already exists.
    pub fn insert(
        &mut self,
        sender: Id256,
        src_token: Id256,
        operation_id: u32,
        order: &SignedMintOrder,
    ) -> Option<SignedMintOrder> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.insert(&key, &operation_id, order)
    }

    /// Returns the signed mint order for the given sender and token, if it exists.
    pub fn get(
        &self,
        sender: Id256,
        src_token: Id256,
        operation_id: u32,
    ) -> Option<SignedMintOrder> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.get(&key, &operation_id)
    }

    /// Returns all the signed mint orders for the given sender and token.
    pub fn get_all(&self, sender: Id256, src_token: Id256) -> Vec<(u32, SignedMintOrder)> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.range(&key).collect()
    }

    /// Removes all signed mint orders.
    pub fn clear(&mut self) {
        self.mint_orders_map.clear();
    }

    pub fn remove(
        &mut self,
        sender: Id256,
        src_token: Id256,
        operation_id: u32,
    ) -> Option<SignedMintOrder> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.remove(&key, &operation_id)
    }
}

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
struct MintOrderKey {
    sender: Id256,
    src_token: Id256,
}

impl MintOrderKey {
    const STORABLE_BYTE_SIZE: usize = Id256::BYTE_SIZE * 2;
}

impl Storable for MintOrderKey {
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        let mut buf = Vec::with_capacity(Self::STORABLE_BYTE_SIZE as _);
        buf.extend_from_slice(&self.sender.0);
        buf.extend_from_slice(&self.src_token.0);
        buf.into()
    }

    fn from_bytes(bytes: Cow<'_, [u8]>) -> Self {
        Self {
            sender: Id256(bytes[..32].try_into().expect("exacted 32 bytes for sender")),
            src_token: Id256(
                bytes[32..64]
                    .try_into()
                    .expect("exacted 32 bytes for src_token"),
            ),
        }
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: Self::STORABLE_BYTE_SIZE as _,
        is_fixed_size: true,
    };
}

#[cfg(test)]
mod tests {
    use candid::Principal;
    use ic_exports::ic_kit::MockContext;
    use ic_stable_structures::stable_structures::DefaultMemoryImpl;
    use ic_stable_structures::{default_ic_memory_manager, MemoryId, Storable, VirtualMemory};
    use minter_did::id256::Id256;

    use super::{MintOrder, MintOrderKey, MintOrders, SignedMintOrder};

    #[test]
    fn mint_order_key_encoding() {
        let mint_order_key = MintOrderKey {
            sender: Id256::from(&Principal::management_canister()),
            src_token: Id256::from(&Principal::anonymous()),
        };

        let decoded = MintOrderKey::from_bytes(mint_order_key.to_bytes());
        assert_eq!(mint_order_key, decoded);
    }

    fn init_context() -> MintOrders<VirtualMemory<DefaultMemoryImpl>> {
        let memory_manager = default_ic_memory_manager();
        MockContext::new().inject();
        MintOrders::new(memory_manager.get(MemoryId::new(0)))
    }

    #[test]
    fn insert_mint_order() {
        let mut orders = init_context();

        let sender = Id256::from(&Principal::management_canister());
        let src_token = Id256::from(&Principal::anonymous());
        let operation_id = 0;

        let order = SignedMintOrder(vec![0; MintOrder::SIGNED_ENCODED_DATA_SIZE]);

        assert!(orders
            .insert(sender, src_token, operation_id, &order)
            .is_none());
        assert!(orders
            .insert(sender, src_token, operation_id, &order)
            .is_some());
        assert_eq!(orders.get(sender, src_token, operation_id), Some(order));
    }

    #[test]
    fn test_should_remove_mint_order() {
        let mut orders = init_context();

        let sender = Id256::from(&Principal::management_canister());
        let src_token = Id256::from(&Principal::anonymous());
        let operation_id = 0;

        let order = SignedMintOrder(vec![0; MintOrder::SIGNED_ENCODED_DATA_SIZE]);

        assert!(orders
            .insert(sender, src_token, operation_id, &order)
            .is_none());
        assert!(orders.remove(sender, src_token, operation_id).is_some());
        assert!(orders.get(sender, src_token, operation_id).is_none());
    }

    #[test]
    fn get_all_mint_orders() {
        let mut orders = init_context();

        let sender = Id256::from(&Principal::management_canister());
        let other_sender = Id256::from(&Principal::anonymous());
        let src_token = Id256::from(&Principal::anonymous());
        let other_src_token = Id256::from(&Principal::management_canister());
        let order = SignedMintOrder(vec![0; MintOrder::SIGNED_ENCODED_DATA_SIZE]);

        assert!(orders.insert(sender, src_token, 0, &order).is_none());
        assert!(orders.insert(sender, src_token, 1, &order).is_none());

        assert!(orders.insert(other_sender, src_token, 2, &order).is_none());
        assert!(orders.insert(other_sender, src_token, 3, &order).is_none());

        assert!(orders.insert(sender, other_src_token, 4, &order).is_none());
        assert!(orders.insert(sender, other_src_token, 5, &order).is_none());

        assert_eq!(
            orders.get_all(sender, src_token),
            vec![(0, order.clone()), (1, order.clone())]
        );
        assert_eq!(
            orders.get_all(other_sender, src_token),
            vec![(2, order.clone()), (3, order.clone())]
        );
        assert_eq!(
            orders.get_all(sender, other_src_token),
            vec![(4, order.clone()), (5, order)]
        );
    }
}