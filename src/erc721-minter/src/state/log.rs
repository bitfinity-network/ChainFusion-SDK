use std::borrow::Cow;

use candid::{Decode, Encode};
use ic_log::{init_log, LogSettings, LoggerConfig};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{Bound, CellStructure, StableCell, Storable, VirtualMemory};

use crate::memory::{LOGGER_SETTINGS_MEMORY_ID, MEMORY_MANAGER};

#[derive(Debug, Default, Clone)]
pub struct StorableLogSettings(pub LogSettings);

impl Storable for StorableLogSettings {
    const BOUND: Bound = Bound::Unbounded;

    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::from(Encode!(&self.0).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Self(Decode!(&bytes, LogSettings).unwrap())
    }
}

/// Handles the runtime logger configuration
pub struct LoggerConfigService {
    settings: StableCell<StorableLogSettings, VirtualMemory<DefaultMemoryImpl>>,
    config: Option<LoggerConfig>,
}

impl Default for LoggerConfigService {
    fn default() -> Self {
        let settings = LogSettings::default();
        Self {
            settings: StableCell::new(
                MEMORY_MANAGER.with(|mm| mm.get(LOGGER_SETTINGS_MEMORY_ID)),
                StorableLogSettings(settings),
            )
            .expect("failed to init default logger settings"),
            config: None,
        }
    }
}

impl LoggerConfigService {
    /// Initialize a new LoggerConfigService. Must be called just once
    pub fn init(&mut self, log_settings: LogSettings) {
        self.settings
            .set(StorableLogSettings(log_settings))
            .expect("failed to init logger settings");

        // get settings and init log
        let log_settings = self.settings.get().0.clone();
        let logger_config = init_log(&log_settings).expect("failed to init logger");
        self.config.replace(logger_config);
    }
}
