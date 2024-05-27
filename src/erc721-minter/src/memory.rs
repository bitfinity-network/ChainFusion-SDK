use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{IcMemoryManager, MemoryId};

pub const CONFIG_MEMORY_ID: MemoryId = MemoryId::new(0);
pub const PENDING_TASKS_MEMORY_ID: MemoryId = MemoryId::new(1);
pub const SIGNER_MEMORY_ID: MemoryId = MemoryId::new(2);
pub const MINT_ORDERS_MEMORY_ID: MemoryId = MemoryId::new(3);
pub const LOGGER_SETTINGS_MEMORY_ID: MemoryId = MemoryId::new(4);

thread_local! {
    pub static MEMORY_MANAGER: IcMemoryManager<DefaultMemoryImpl> = IcMemoryManager::init(DefaultMemoryImpl::default());
}
