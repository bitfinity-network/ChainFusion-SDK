use std::time::{SystemTime, UNIX_EPOCH};

use candid::{CandidType, Deserialize, Principal};
use did::codec;
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{BTreeMapStructure, StableBTreeMap, Storable, VirtualMemory};

use crate::memory::{CANISTER_REGISTRY_MEMORY_ID, MEMORY_MANAGER};
use crate::types::{CanisterArgs, CanisterType};

#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum CanisterStatus {
    Deployed,
    Upgraded,
    Reinstalled,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
/// `CanisterInfo` represents the metadata associated with a canister in the registry.
///
/// - `canister_type`: The type of the canister.
/// - `with_args`: The arguments used to create the canister.
/// - `hash`: The hash of the canister's code.
/// - `status`: The current status of the canister (deployed, upgraded, reinstalled).
/// - `timestamp`: The timestamp when the canister was registered.
pub struct CanisterInfo {
    pub canister_type: CanisterType,
    with_args: CanisterArgs,
    hash: String,
    status: CanisterStatus,
    timestamp: u64,
}

impl Storable for CanisterInfo {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        codec::encode(&self).into()
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        codec::decode(&bytes)
    }

    const BOUND: ic_stable_structures::Bound = ic_stable_structures::Bound::Unbounded;
}

pub struct Registry {
    canisters: StableBTreeMap<Principal, CanisterInfo, VirtualMemory<DefaultMemoryImpl>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            canisters: StableBTreeMap::new(
                MEMORY_MANAGER.with(|mm| mm.get(CANISTER_REGISTRY_MEMORY_ID)),
            ),
        }
    }
}

impl Registry {
    pub fn register_canister(&mut self, principal: Principal, hash: String, args: CanisterArgs) {
        let timestamp = time_secs();

        let canister_type = args._type();

        let info = CanisterInfo {
            canister_type,
            hash,
            status: CanisterStatus::Deployed,
            timestamp,
            with_args: args,
        };

        self.canisters.insert(principal, info);
    }

    pub fn get_canister_info(&self, principal: &Principal) -> Option<CanisterInfo> {
        self.canisters.get(principal)
    }

    pub fn get_all_canisters(&self) -> Vec<(Principal, CanisterInfo)> {
        self.canisters.iter().map(|(k, v)| (k, v.clone())).collect()
    }

    pub fn update_canister_status(
        &mut self,
        principal: Principal,
        status: CanisterStatus,
        hash: Option<String>,
    ) {
        if let Some(mut info) = self.canisters.get(&principal) {
            info.status = status;
            if let Some(new_hash) = hash {
                info.hash = new_hash;
            }

            info.timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();

            self.canisters.insert(principal, info);
        }
    }

    pub fn get_canister_principal(&self, canister_type: CanisterType) -> Option<Principal> {
        self.canisters
            .iter()
            .find(|(_, info)| info.canister_type == canister_type)
            .map(|(principal, _)| principal)
    }

    pub fn remove_canister(&mut self, principal: &Principal) {
        self.canisters.remove(principal);
    }

    pub fn clear(&mut self) {
        self.canisters.clear();
    }
}

/// returns the timestamp in seconds
#[inline]
pub fn time_secs() -> u64 {
    #[cfg(not(target_family = "wasm"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("get current timestamp error")
            .as_secs()
    }

    // ic::time() return the nano_sec, we need to change it to sec.
    #[cfg(target_family = "wasm")]
    (ic_exports::ic_kit::ic::time() / crate::constant::E_9)
}

#[cfg(test)]
mod tests {

    use candid::Principal;
    use erc20_minter::state::Settings;
    use ethers_core::types::H256;
    use minter_did::init::InitData;

    use crate::state::registry::{CanisterStatus, Registry};
    use crate::types::{CanisterArgs, CanisterType};

    #[test]
    fn test_register_canister() {
        let mut registry = Registry::default();
        let principal = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        let hash = "test_hash".to_string();
        let args = CanisterArgs::ERC20(Settings {
            base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
            wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
            signing_strategy: icrc2_minter::SigningStrategy::Local {
                private_key: *H256::random().as_fixed_bytes(),
            },
            log_settings: None,
        });

        registry.register_canister(principal, hash.clone(), args);

        let info = registry.get_canister_info(&principal).unwrap();
        assert_eq!(info.canister_type, CanisterType::ERC20);
        assert_eq!(info.hash, hash);
        assert!(matches!(info.status, CanisterStatus::Deployed));
    }

    #[test]
    fn test_get_all_canisters() {
        let mut registry = Registry::default();
        let principal1 = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        let principal2 = Principal::from_text("renrk-eyaaa-aaaaa-aaada-cai").unwrap();

        registry.register_canister(
            principal1,
            "hash1".to_string(),
            CanisterArgs::ERC20(Settings {
                base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );
        registry.register_canister(
            principal2,
            "hash2".to_string(),
            CanisterArgs::ICRC(InitData {
                owner: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                evm_principal: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );

        let all_canisters = registry.get_all_canisters();
        assert_eq!(all_canisters.len(), 2);
        assert!(all_canisters.iter().any(|(p, _)| *p == principal1));
        assert!(all_canisters.iter().any(|(p, _)| *p == principal2));
    }

    #[test]
    fn test_update_canister_status() {
        let mut registry = Registry::default();
        let principal = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        registry.register_canister(
            principal,
            "old_hash".to_string(),
            CanisterArgs::ERC20(Settings {
                base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );

        registry.update_canister_status(
            principal,
            CanisterStatus::Upgraded,
            Some("new_hash".to_string()),
        );

        let info = registry.get_canister_info(&principal).unwrap();
        assert!(matches!(info.status, CanisterStatus::Upgraded));
        assert_eq!(info.hash, "new_hash");
    }

    #[test]
    fn test_get_canister_principal() {
        let mut registry = Registry::default();
        let icrc_principal = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        let erc_principal = Principal::from_text("renrk-eyaaa-aaaaa-aaada-cai").unwrap();

        registry.register_canister(
            icrc_principal,
            "hash1".to_string(),
            CanisterArgs::ICRC(InitData {
                owner: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                evm_principal: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );
        registry.register_canister(
            erc_principal,
            "hash2".to_string(),
            CanisterArgs::ERC20(Settings {
                base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );

        assert_eq!(
            registry.get_canister_principal(CanisterType::ICRC),
            Some(icrc_principal)
        );
        assert_eq!(
            registry.get_canister_principal(CanisterType::ERC20),
            Some(erc_principal)
        );
    }

    #[test]
    fn test_remove_canister() {
        let mut registry = Registry::default();
        let principal = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        registry.register_canister(
            principal,
            "hash".to_string(),
            CanisterArgs::ERC20(Settings {
                base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );

        registry.remove_canister(&principal);

        assert!(registry.get_canister_info(&principal).is_none());
    }

    #[test]
    fn test_clear() {
        let mut registry = Registry::default();
        let principal1 = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();
        let principal2 = Principal::from_text("renrk-eyaaa-aaaaa-aaada-cai").unwrap();

        registry.register_canister(
            principal1,
            "hash1".to_string(),
            CanisterArgs::ERC20(Settings {
                base_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                wrapped_evm_link: minter_contract_utils::evm_link::EvmLink::Http("".to_owned()),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );
        registry.register_canister(
            principal2,
            "hash2".to_string(),
            CanisterArgs::ICRC(InitData {
                owner: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                evm_principal: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap(),
                signing_strategy: icrc2_minter::SigningStrategy::Local {
                    private_key: *H256::random().as_fixed_bytes(),
                },
                log_settings: None,
            }),
        );

        registry.clear();

        assert!(registry.get_all_canisters().is_empty());
    }
}
