use std::borrow::Cow;

use ic_stable_structures::stable_structures::Memory;
use ic_stable_structures::{Bound, MultimapStructure as _, StableMultimap, Storable};
use minter_did::erc721_mint_order::ERC721SignedMintOrder;
use minter_did::id256::Id256;

pub struct MintOrders<M: Memory> {
    mint_orders_map: StableMultimap<MintOrderKey, u32, ERC721SignedMintOrder, M>,
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
        order: &ERC721SignedMintOrder,
    ) -> Option<ERC721SignedMintOrder> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.insert(&key, &operation_id, order)
    }

    /// Returns the signed mint order for the given sender and token, if it exists.
    pub fn get(
        &self,
        sender: Id256,
        src_token: Id256,
        operation_id: u32,
    ) -> Option<ERC721SignedMintOrder> {
        let key = MintOrderKey { sender, src_token };
        self.mint_orders_map.get(&key, &operation_id)
    }

    /// Returns all the signed mint orders for the given sender and token.
    pub fn get_all(&self, sender: Id256, src_token: Id256) -> Vec<(u32, ERC721SignedMintOrder)> {
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
    ) -> Option<ERC721SignedMintOrder> {
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
    use minter_did::erc721_mint_order::{ERC721MintOrder, ERC721SignedMintOrder};
    use minter_did::id256::Id256;

    use super::{MintOrderKey, MintOrders};

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

        let order = ERC721SignedMintOrder(vec![0; ERC721MintOrder::SIGNED_ENCODED_DATA_SIZE]);

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

        let order = ERC721SignedMintOrder(vec![0; ERC721MintOrder::SIGNED_ENCODED_DATA_SIZE]);

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
        let order = ERC721SignedMintOrder(vec![0; ERC721MintOrder::SIGNED_ENCODED_DATA_SIZE]);

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
