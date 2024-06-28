use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{IcMemoryManager, MemoryId};

pub const CONFIG_MEMORY_ID: MemoryId = MemoryId::new(1);
pub const TX_SIGNER_MEMORY_ID: MemoryId = MemoryId::new(2);

pub const CANISTER_REGISTRY_MEMORY_ID: MemoryId = MemoryId::new(3);

thread_local! {
    pub static MEMORY_MANAGER: IcMemoryManager<DefaultMemoryImpl> = IcMemoryManager::init(DefaultMemoryImpl::default());
}
