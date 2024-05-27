use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{IcMemoryManager, MemoryId};

pub const PENDING_TASKS_MEMORY_ID: MemoryId = MemoryId::new(1);
pub const SIGNER_MEMORY_ID: MemoryId = MemoryId::new(2);
pub const MINT_ORDERS_MEMORY_ID: MemoryId = MemoryId::new(3);
pub const BURN_REQUEST_MEMORY_ID: MemoryId = MemoryId::new(4);
pub const NFT_STORE_MEMORY_ID: MemoryId = MemoryId::new(5);

thread_local! {
    pub static MEMORY_MANAGER: IcMemoryManager<DefaultMemoryImpl> = IcMemoryManager::init(DefaultMemoryImpl::default());
}
