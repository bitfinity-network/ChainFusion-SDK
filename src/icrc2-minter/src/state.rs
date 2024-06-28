use access_list::AccessList;
use candid::Principal;
use config::Config;
pub use eth_signer::sign_strategy::{SigningStrategy, TransactionSigner};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{default_ic_memory_manager, VirtualMemory};

use self::log::LoggerConfigService;
use self::signer::SignerInfo;
use crate::constant::ACCESS_LIST_MEMORY_ID;

mod access_list;
mod config;
pub mod log;
mod signer;

/// State of a minter canister.
pub(crate) struct State {
    /// Minter canister configuration.
    pub config: Config,

    /// Transaction signing info.
    pub signer: SignerInfo,

    pub logger_config_service: LoggerConfigService,

    pub access_list: AccessList<VirtualMemory<DefaultMemoryImpl>>,
}

impl Default for State {
    fn default() -> Self {
        let memory_manager = default_ic_memory_manager();
        Self {
            config: Config::default(),
            signer: SignerInfo::default(),
            logger_config_service: LoggerConfigService::default(),
            access_list: AccessList::new(memory_manager.get(ACCESS_LIST_MEMORY_ID)),
        }
    }
}

impl State {
    /// Clear the state and set initial data from settings.
    pub fn reset(&mut self, settings: Settings) {
        self.signer
            .reset(settings.signing_strategy.clone(), 0)
            .expect("failed to set signer");
        self.config.reset(settings);
    }
}

/// State settings.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub owner: Principal,
    pub evm_principal: Principal,
    pub signing_strategy: SigningStrategy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            owner: Principal::anonymous(),
            evm_principal: Principal::anonymous(),
            signing_strategy: SigningStrategy::Local {
                private_key: [218u8; 32],
            },
        }
    }
}
