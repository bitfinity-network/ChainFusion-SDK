use std::borrow::Cow;
use std::cell::RefCell;

use candid::{CandidType, Deserialize, Principal};
use did::codec;
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{CellStructure, StableCell, Storable, VirtualMemory};

use super::Settings;
use crate::memory::{CONFIG_MEMORY_ID, MEMORY_MANAGER};

/// Minter canister configuration.
#[derive(Default, Clone)]
pub struct Config {}

impl Config {
    /// Clear configuration and initialize it with data from `settings`.
    pub fn reset(&mut self, settings: Settings) {
        let new_data = ConfigData {
            owner: settings.owner,
        };

        self.update_data(|data| *data = new_data);
    }

    /// Returns principal of canister owner.
    pub fn get_owner(&self) -> Principal {
        self.with_data(|data| data.get().owner)
    }

    /// Sets a new principal for canister owner.
    pub fn set_owner(&mut self, owner: Principal) {
        self.update_data(|data| data.owner = owner);
    }

    fn with_data<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&StableCell<ConfigData, VirtualMemory<DefaultMemoryImpl>>) -> T,
    {
        CONFIG_CELL.with(|cell| f(&mut cell.borrow()))
    }

    fn with_mut_data<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut StableCell<ConfigData, VirtualMemory<DefaultMemoryImpl>>) -> T,
    {
        CONFIG_CELL.with(|cell| f(&mut cell.borrow_mut()))
    }

    fn update_data<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut ConfigData) -> T,
    {
        self.with_mut_data(|data| {
            let mut old_data = data.get().clone();
            let result = f(&mut old_data);
            data.set(old_data)
                .expect("failed to update config stable memory data");
            result
        })
    }
}

#[derive(Debug, Clone, Deserialize, CandidType, PartialEq, Eq, serde::Serialize)]
pub struct ConfigData {
    pub owner: Principal,
}

impl Default for ConfigData {
    fn default() -> Self {
        Self {
            owner: Principal::management_canister(),
        }
    }
}

impl Storable for ConfigData {
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        codec::encode(&self).into()
    }

    fn from_bytes(bytes: Cow<'_, [u8]>) -> Self {
        codec::decode(bytes.as_ref())
    }

    const BOUND: ic_stable_structures::Bound = ic_stable_structures::Bound::Unbounded;
}

thread_local! {
    static CONFIG_CELL: RefCell<StableCell<ConfigData, VirtualMemory<DefaultMemoryImpl>>> = {
        RefCell::new(StableCell::new(MEMORY_MANAGER.with(|mm| mm.get(CONFIG_MEMORY_ID)), ConfigData::default())
            .expect("stable memory config initialization failed"))
    };
}

#[cfg(test)]
mod tests {
    use candid::Principal;
    use eth_signer::sign_strategy::SigningStrategy;
    use ic_exports::ic_kit::MockContext;
    use ic_stable_structures::Storable;

    use super::*;
    use crate::state::config::ConfigData;
    use crate::state::Settings;

    fn get_config() -> Config {
        MockContext::new().inject();
        let mut config = Config::default();
        config.reset(Settings::default());
        config
    }

    #[test]
    fn config_serialization() {
        let config = ConfigData::default();
        let encoded = config.to_bytes();
        let decoded = ConfigData::from_bytes(encoded);
        assert_eq!(config, decoded);
    }

    #[test]
    fn reset_should_update_config() {
        let mut config = get_config();

        let settings = Settings {
            owner: Principal::management_canister(),

            signing_strategy: SigningStrategy::Local {
                private_key: [1u8; 32],
            },
        };

        config.reset(settings.clone());

        assert_eq!(config.get_owner(), settings.owner);
    }

    #[test]
    fn config_data_stored_after_set() {
        let mut config = get_config();

        config.set_owner(Principal::management_canister());

        assert_eq!(config.get_owner(), Principal::management_canister());
    }
}
