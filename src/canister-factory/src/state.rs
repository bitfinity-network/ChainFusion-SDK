use candid::Principal;
use config::Config;
use eth_signer::sign_strategy::SigningStrategy;
use registry::Registry;
use signer::SignerInfo;

mod config;
mod registry;
pub mod signer;

pub use registry::{CanisterInfo, CanisterStatus};

/// State of a minter canister.
#[derive(Default)]
pub struct State {
    /// Minter canister configuration.
    pub config: Config,

    /// Transaction signing info.
    pub signer: SignerInfo,

    /// Registry of deployed canisters.
    registry: Registry,
}

impl State {
    /// Clear the state and set initial data from settings.
    pub fn reset(&mut self, settings: Settings) {
        self.signer
            .reset(settings.signing_strategy.clone(), 0)
            .expect("failed to set signer");
        self.config.reset(settings);
        self.registry.clear();
    }

    /// Returns the registry of deployed canisters.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn mut_registry(&mut self) -> &mut Registry {
        &mut self.registry
    }
}

/// State settings.
#[derive(Debug, Clone)]
pub struct Settings {
    pub owner: Principal,
    pub signing_strategy: SigningStrategy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            owner: Principal::management_canister(),
            signing_strategy: SigningStrategy::Local {
                private_key: [218u8; 32],
            },
        }
    }
}
