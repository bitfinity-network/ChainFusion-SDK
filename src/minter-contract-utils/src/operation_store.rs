//! Abstract stable storage for user-initiated operations in bridge canisters. It can be used
//! to track an operation status and retrieve all operations for a given user ETH wallet.
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::{Display, Formatter};

use candid::{CandidType, Decode, Deserialize, Encode};
use did::H160;
use ic_stable_structures::stable_structures::{DefaultMemoryImpl, Memory};
use ic_stable_structures::{
    BTreeMapStructure, Bound, CachedStableBTreeMap, CellStructure, IcMemoryManager, MemoryId,
    StableBTreeMap, StableCell, Storable, VirtualMemory,
};
use serde::Serialize;

const DEFAULT_CACHE_SIZE: u32 = 1000;
const DEFAULT_MAX_REQUEST_COUNT: u64 = 100_000;

pub const OPERATION_ID_MEMORY_ID: MemoryId = MemoryId::new(253);
thread_local! {
    static MEMORY_MANAGER: IcMemoryManager<DefaultMemoryImpl> = IcMemoryManager::init(DefaultMemoryImpl::default());
    static OPERATION_ID_COUNTER: RefCell<StableCell<u64, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableCell::new(MEMORY_MANAGER.with(|mm| mm.get(OPERATION_ID_MEMORY_ID)), 0)
            .expect("failed to initialize operation id cell"));
}

/// Unique ID of an operation.
#[derive(
    Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, CandidType, Deserialize, Serialize, Hash,
)]
pub struct MinterOperationId(u64);

impl MinterOperationId {
    fn next() -> Self {
        let id = OPERATION_ID_COUNTER.with(|cell| {
            let mut cell = cell.borrow_mut();
            let id = *cell.get();
            cell.set(id + 1).expect("failed to update nonce counter");
            id
        });
        Self(id)
    }

    /// Returns a unique `nonce` value for given operation ID.
    pub fn nonce(&self) -> u32 {
        (self.0 % u32::MAX as u64) as u32
    }
}

impl Storable for MinterOperationId {
    fn to_bytes(&self) -> Cow<[u8]> {
        self.0.to_bytes()
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Self(u64::from_bytes(bytes))
    }

    const BOUND: Bound = <u64 as Storable>::BOUND;
}

impl Display for MinterOperationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, CandidType, Deserialize)]
struct OperationStoreEntry<P>
where
    P: CandidType,
{
    dst_address: H160,
    payload: P,
}

impl<P> Storable for OperationStoreEntry<P>
where
    P: CandidType + Clone + for<'de> Deserialize<'de>,
{
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).expect("failed to encode deposit request"))
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).expect("failed to decode deposit request")
    }

    const BOUND: Bound = Bound::Unbounded;
}

#[derive(Default, Debug, Clone, CandidType, Deserialize)]
struct OperationIdList(Vec<MinterOperationId>);

impl Storable for OperationIdList {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).expect("failed to encode deposit request"))
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).expect("failed to decode deposit request")
    }

    const BOUND: Bound = Bound::Unbounded;
}

/// Parameters of the [`MinterOperationStore`].
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct MinterOperationStoreOptions {
    max_operations_count: u64,
    cache_size: u32,
}

impl Default for MinterOperationStoreOptions {
    fn default() -> Self {
        Self {
            max_operations_count: DEFAULT_MAX_REQUEST_COUNT,
            cache_size: DEFAULT_CACHE_SIZE,
        }
    }
}

pub trait MinterOperation {
    fn is_complete(&self) -> bool;
}

/// A structure to store user-initiated operations in IC stable memory.
///
/// Every operation in the store is attached to a ETH wallet address that initiated the operation.
/// And a list of operations for the given wallet can be retrieved by the [`get_for_address`] method.
///
/// It stores a limited number of latest operations and their information, dropping old operations.
/// The maximum number of operations stored can be configured with `options`.
pub struct MinterOperationStore<M, P>
where
    M: Memory,
    P: MinterOperation + CandidType + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
{
    incomplete_operations: CachedStableBTreeMap<MinterOperationId, OperationStoreEntry<P>, M>,
    operations_log: StableBTreeMap<MinterOperationId, OperationStoreEntry<P>, M>,
    address_operation_map: StableBTreeMap<H160, OperationIdList, M>,
    max_operation_log_size: u64,
}

impl<M, P> MinterOperationStore<M, P>
where
    M: Memory,
    P: MinterOperation + CandidType + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static,
{
    /// Creates a new instance of the store.
    pub fn with_memory(
        curr_operations_memory: M,
        operations_log_memory: M,
        map_memory: M,
        options: Option<MinterOperationStoreOptions>,
    ) -> Self {
        let options = options.unwrap_or_default();
        Self {
            incomplete_operations: CachedStableBTreeMap::new(
                curr_operations_memory,
                options.cache_size,
            ),
            operations_log: StableBTreeMap::new(operations_log_memory),
            address_operation_map: StableBTreeMap::new(map_memory),
            max_operation_log_size: options.max_operations_count,
        }
    }

    /// Initializes a new operation with the given payload for the given ETH wallet address
    /// and stores it.
    pub fn new_operation(&mut self, dst_address: H160, payload: P) -> MinterOperationId {
        let id = MinterOperationId::next();
        let entry = OperationStoreEntry {
            dst_address: dst_address.clone(),
            payload,
        };

        log::trace!("Operation {id} is created.");

        if entry.payload.is_complete() {
            self.move_to_log(id, entry);
        } else {
            self.incomplete_operations.insert(id, entry);
        }

        let mut ids = self
            .address_operation_map
            .get(&dst_address)
            .unwrap_or_default();
        ids.0.push(id);
        self.address_operation_map.insert(dst_address, ids);

        id
    }

    /// Retrieves an operation by its ID.
    pub fn get(&self, operation_id: MinterOperationId) -> Option<P> {
        self.get_with_id(operation_id).map(|(_, p)| p)
    }

    fn get_with_id(&self, operation_id: MinterOperationId) -> Option<(MinterOperationId, P)> {
        self.incomplete_operations
            .get(&operation_id)
            .or_else(|| self.operations_log.get(&operation_id))
            .map(|entry| (operation_id, entry.payload))
    }

    /// Retrieves all operations for the given ETH wallet address,
    /// starting from `offset` returning a max of `count` items
    /// If `offset` is `None`, it starts from the beginning.
    /// If `count` is `None`, it returns all operations.
    pub fn get_for_address(
        &self,
        dst_address: &H160,
        offset: Option<usize>,
        count: Option<usize>,
    ) -> Vec<(MinterOperationId, P)> {
        log::trace!("Operation store contains {} active operations, {} operations in log, {} entries in the map. Value for address {}: {:?}", self.incomplete_operations.len(), self.operations_log.len(), self.address_operation_map.len(), hex::encode(dst_address.0), self.address_operation_map.get(dst_address));
        self.address_operation_map
            .get(dst_address)
            .unwrap_or_default()
            .0
            .into_iter()
            .filter_map(|id| self.get_with_id(id))
            .skip(offset.unwrap_or(0))
            .take(count.unwrap_or(usize::MAX))
            .collect()
    }

    /// Update the payload of the operation with the given id. If no operation with the given ID
    /// is found, nothing is done (except an error message in the log).
    pub fn update(&mut self, operation_id: MinterOperationId, payload: P) {
        let Some(mut entry) = self.incomplete_operations.get(&operation_id) else {
            log::error!("Cannot update operation {operation_id} status: not found");
            return;
        };

        entry.payload = payload;

        if entry.payload.is_complete() {
            self.move_to_log(operation_id, entry);
        } else {
            self.incomplete_operations.insert(operation_id, entry);
        }
    }

    fn move_to_log(&mut self, operation_id: MinterOperationId, entry: OperationStoreEntry<P>) {
        self.incomplete_operations.remove(&operation_id);
        self.operations_log.insert(operation_id, entry);

        log::trace!("Operation {operation_id} is marked as complete and moved to the log.");

        if self.operations_log.len() > self.max_operation_log_size() {
            self.remove_oldest();
        }
    }

    fn max_operation_log_size(&self) -> u64 {
        self.max_operation_log_size
    }

    fn remove_oldest(&mut self) {
        if let Some((id, oldest)) = self.operations_log.iter().next() {
            self.operations_log.remove(&id);
            let mut ids = self
                .address_operation_map
                .get(&oldest.dst_address)
                .unwrap_or_default();
            let count_before = ids.0.len();
            ids.0.retain(|stored_id| *stored_id != id);

            if ids.0.len() != count_before {
                if ids.0.is_empty() {
                    self.address_operation_map.remove(&oldest.dst_address);
                } else {
                    self.address_operation_map.insert(oldest.dst_address, ids);
                }
            }

            log::trace!("Operation {id} is evicted from the operation log");
        }
    }
}

#[cfg(test)]
mod tests {
    use ic_stable_structures::VectorMemory;

    use super::*;

    const COMPLETE: u32 = u32::MAX;
    impl MinterOperation for u32 {
        fn is_complete(&self) -> bool {
            *self == COMPLETE
        }
    }

    fn test_store(max_operations: u64) -> MinterOperationStore<VectorMemory, u32> {
        MinterOperationStore::with_memory(
            VectorMemory::default(),
            VectorMemory::default(),
            VectorMemory::default(),
            Some(MinterOperationStoreOptions {
                max_operations_count: max_operations,
                cache_size: DEFAULT_CACHE_SIZE,
            }),
        )
    }

    fn eth_address(seed: u8) -> H160 {
        H160::from([seed; H160::BYTE_SIZE])
    }

    #[test]
    fn nonce_should_increment_with_id() {
        const CHECKS_NUM: usize = 100;
        let id1 = MinterOperationId::next();
        for _ in 0..CHECKS_NUM {
            let id2 = MinterOperationId::next();
            assert_ne!(id1, id2);
            assert_ne!(id1.nonce(), id2.nonce());
        }
    }

    #[test]
    fn nonce_should_not_overflow() {
        OPERATION_ID_COUNTER.with(|cell| {
            let mut cell = cell.borrow_mut();
            let id = u32::MAX as u64 * 3 - 3;
            cell.set(id).unwrap();
        });

        const CHECKS_NUM: usize = 100;
        let id1 = MinterOperationId::next();
        for _ in 0..CHECKS_NUM {
            let id2 = MinterOperationId::next();
            assert_ne!(id1, id2);
            assert_ne!(id1.nonce(), id2.nonce());
        }
    }

    #[test]
    fn operations_log_limit() {
        const LIMIT: u64 = 10;
        const COUNT: u64 = 42;

        let mut store = test_store(LIMIT);

        for i in 0..COUNT {
            store.new_operation(eth_address(i as u8), COMPLETE);
        }

        assert_eq!(store.operations_log.len(), LIMIT);
        assert_eq!(store.address_operation_map.len(), LIMIT);

        for i in 0..(COUNT - LIMIT) {
            assert!(store
                .get_for_address(&eth_address(i as u8), None, None)
                .is_empty());
        }

        for i in (COUNT - LIMIT)..COUNT {
            assert_eq!(
                store
                    .get_for_address(&eth_address(i as u8), None, None)
                    .len(),
                1,
            );
        }
    }

    #[test]
    fn should_get_page_for_operations() {
        const LIMIT: u64 = 100;
        const COUNT: u64 = 42;

        let mut store = test_store(LIMIT);

        for _ in 0..COUNT {
            store.new_operation(eth_address(0), COMPLETE);
        }

        assert_eq!(store.operations_log.len(), COUNT);

        // No offset, with count
        let page = store.get_for_address(&eth_address(0), None, Some(10));
        assert_eq!(page.len(), 10);
        // No offset with count > total
        let page = store.get_for_address(&eth_address(0), None, Some(120));
        assert_eq!(page.len() as u64, COUNT);

        // Offset with count
        let page = store.get_for_address(&eth_address(0), Some(20), Some(15));
        assert_eq!(page.len(), 15);

        // Offset with count beyond total
        let page = store.get_for_address(&eth_address(0), Some(100), Some(10));
        assert!(page.is_empty());

        // Offset without count
        let page = store.get_for_address(&eth_address(0), Some(20), None);
        assert_eq!(page.len(), COUNT as usize - 20usize);

        // No offset, no count
        let page = store.get_for_address(&eth_address(0), None, None);
        assert_eq!(page.len(), COUNT as usize);
    }

    #[test]
    fn operations_limit_with_same_address() {
        const LIMIT: u64 = 10;
        const COUNT: u64 = 42;

        let mut store = test_store(LIMIT);

        for _ in 0..COUNT {
            store.new_operation(eth_address(1), COMPLETE);
        }

        assert_eq!(store.operations_log.len(), LIMIT);
        assert_eq!(store.address_operation_map.len(), 1);

        assert_eq!(
            store.get_for_address(&eth_address(1), None, None).len(),
            LIMIT as usize
        );
    }

    #[test]
    fn incomplete_operations_are_not_removed() {
        const LIMIT: u64 = 10;
        const COUNT: u64 = 42;

        let mut store = test_store(LIMIT);

        for _ in 0..COUNT {
            let id = store.new_operation(eth_address(1), 1);
            store.update(id, 2);
        }

        assert_eq!(store.operations_log.len(), 0);
        assert_eq!(store.incomplete_operations.len(), COUNT);
        assert_eq!(store.address_operation_map.len(), 1);

        assert_eq!(
            store.get_for_address(&eth_address(1), None, None).len(),
            COUNT as usize
        );
    }

    #[test]
    fn operations_are_moved_to_log_on_completion() {
        const LIMIT: u64 = 10;
        const COUNT: u64 = 42;

        let mut store = test_store(LIMIT);

        let mut ids = vec![];
        for i in 0..COUNT {
            ids.push(store.new_operation(eth_address(i as u8), 1));
        }

        for id in ids {
            let count_before = store.incomplete_operations.len();
            store.update(id, COMPLETE);
            let count_after = store.incomplete_operations.len();
            assert_eq!(count_after, count_before - 1);
        }

        assert_eq!(store.operations_log.len(), LIMIT);
        assert_eq!(store.incomplete_operations.len(), 0);
        assert_eq!(store.address_operation_map.len(), LIMIT);
    }
}
