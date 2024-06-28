use std::cell::RefCell;
use std::rc::Rc;

use candid::{CandidType, Principal};
use did::{H160, H256};
use eth_signer::sign_strategy::TransactionSigner;
use ethers_core::abi::Token;
use ethers_core::utils::keccak256;
use evm_canister_client::{EvmCanisterClient, IcCanisterClient};
use ic_canister::{init, query, update, Canister, Idl, MethodType, PreUpdate};
use ic_exports::ic_cdk::api::management_canister::main::{
    CanisterIdRecord, CanisterInstallMode, CanisterStatusType, CreateCanisterArgument,
    InstallCodeArgument,
};
use ic_exports::ic_cdk::api::management_canister::provisional::CanisterSettings;
use ic_exports::ic_kit::ic;
use ic_metrics::{Metrics, MetricsStorage};
use icrc2_minter::SigningStrategy;

use crate::error::{Result, UpgraderError};
use crate::hash;
use crate::management::Management;
use crate::state::{CanisterInfo, CanisterStatus, Settings, State};
use crate::types::*;

pub const CREATE_CYCLES: u128 = 2_000_000_000;

/// A canister to transfer funds between IC token canisters and EVM canister contracts.
#[derive(Canister, Clone)]
pub struct CanisterFactory {
    #[id]
    id: Principal,
}

#[derive(CandidType)]
pub struct UpgraderInitData {
    pub owner: Principal,
    pub signing_strategy: SigningStrategy,
}

impl PreUpdate for CanisterFactory {
    fn pre_update(&self, _method_name: &str, _method_type: MethodType) {}
}

impl CanisterFactory {
    #[init]
    fn init(&self, init: UpgraderInitData) {
        let state = get_state();
        let mut state = state.borrow_mut();

        check_anonymous_principal(init.owner).expect("anonymous principal not allowed");

        let settings = Settings {
            owner: init.owner,
            signing_strategy: init.signing_strategy,
        };

        state.reset(settings);
    }

    /// Returns principal of canister owner.
    #[query]
    pub fn get_owner(&self) -> Principal {
        get_state().borrow().config.get_owner()
    }

    /// set_owner inspect_message check
    pub(crate) fn set_owner_inspect_message_check(
        principal: Principal,
        owner: Principal,
        state: &State,
    ) -> Result {
        check_anonymous_principal(owner)?;
        inspect_check_is_owner(principal, state)?;

        Ok(())
    }

    /// Sets a new principal for canister owner.
    ///
    /// This method should be called only by current owner,
    /// else `Error::NotAuthorised` will be returned.
    #[update]
    pub fn set_owner(&mut self, owner: Principal) -> Result {
        let state = get_state();
        let mut state = state.borrow_mut();

        CanisterFactory::set_owner_inspect_message_check(ic::caller(), owner, &state)?;
        state.config.set_owner(owner);

        Ok(())
    }

    /// Returns evm_address of the minter canister.
    #[update]
    pub async fn get_ethereum_address(&mut self) -> Result<H160> {
        let signer = get_state().borrow().signer.get_transaction_signer();
        let addr = signer
            .get_address()
            .await
            .map_err(|e| UpgraderError::TransactionSignerError(e.to_string()))?;

        Ok(addr)
    }

    /// Deploys a new canister with the provided arguments and WASM code.
    ///
    /// This method performs the following steps:
    /// 1. Validates the provided WASM code and canister type.
    /// 2. Creates a new empty canister using the provided settings.
    /// 3. Installs the WASM code and arguments on the new canister.
    /// 4. Registers the new canister in the state registry.
    ///
    /// # Arguments
    /// * `canister_args` - The arguments for the new canister.
    /// * `wasm` - The WASM code for the new canister.
    /// * `settings` - Optional settings for the new canister.
    /// * `cycles` - Optional cycles to be deposited in the new canister.
    ///
    /// # Returns
    /// The principal ID of the newly deployed canister.
    #[update]
    pub async fn deploy(
        &self,
        canister_args: CanisterArgs,
        wasm: Vec<u8>,
        settings: Option<CanisterSettings>,
        cycles: Option<u128>,
    ) -> Result<Principal> {
        let canister_type = canister_args._type();

        self.validate(&canister_type, wasm.clone())?;
        let args = CreateCanisterArgument { settings };

        let wasm_hash = hash::hash_wasm_module_hex(&wasm);

        let cycles = cycles.unwrap_or(CREATE_CYCLES);

        // Create empty canister
        let canister_id = Management::create_canister(args, cycles).await?;

        let args = canister_args.encode_args()?;

        Self::deploy_canister(canister_id, wasm, args).await?;

        let state = get_state();
        let mut state = state.borrow_mut();

        state
            .mut_registry()
            .register_canister(canister_id.canister_id, wasm_hash, canister_args);

        Ok(canister_id.canister_id)
    }

    /// Upgrades the canister with the provided WASM code.
    ///
    /// This method performs the following steps:
    /// 1. Validates the provided WASM code and canister type.
    /// 2. Stops the existing canister.
    /// 3. Installs the new WASM code and arguments on the canister.
    /// 4. Updates the canister status in the state registry.
    /// 5. Starts the upgraded canister.
    ///
    /// # Arguments
    /// * `canister_type` - The type of the canister to be upgraded.
    /// * `wasm` - The new WASM code for the canister.
    ///
    /// # Returns
    /// A result indicating whether the upgrade was successful.
    #[update]
    pub async fn upgrade(&self, canister_type: CanisterType, wasm: Vec<u8>) -> Result {
        self.validate(&canister_type, wasm.clone())?;

        let wasm_hash = hash::hash_wasm_module_hex(&wasm);

        // Check if the canister is in the registry
        let state = get_state();

        let principal = state
            .borrow()
            .registry()
            .get_canister_principal(canister_type)
            .ok_or(UpgraderError::CanisterNotFound)?;

        // Stop the canister
        Management::stop_canister(principal).await?;

        let arg = candid::encode_args(()).map_err(|e| UpgraderError::CandidError(e.to_string()))?;

        let arg = InstallCodeArgument {
            mode: CanisterInstallMode::Upgrade(None),
            canister_id: principal,
            wasm_module: wasm,
            arg,
        };
        Management::install_code(arg)
            .await
            .map_err(|e| UpgraderError::CanisterUpgradeFailed(e.to_string()))?;

        // Update the registry
        state.borrow_mut().mut_registry().update_canister_status(
            principal,
            CanisterStatus::Upgraded,
            Some(wasm_hash),
        );

        // Start the canister
        Management::start_canister(principal).await?;

        Ok(())
    }

    /// Reinstalls a canister with the provided WASM code and arguments.
    ///
    /// # Arguments
    /// * `canister_args` - The arguments for the canister to be reinstalled.
    /// * `wasm` - The new WASM code for the canister.
    ///
    /// # Returns
    /// A result indicating whether the reinstall was successful.
    #[update]
    pub async fn reinstall(&self, canister_args: CanisterArgs, wasm: Vec<u8>) -> Result {
        let canister_type = canister_args._type();

        self.validate(&canister_type, wasm.clone())?;

        let wasm_hash = hash::hash_wasm_module_hex(&wasm);

        // Check if the canister is in the registry
        let state = get_state();

        let principal = state
            .borrow()
            .registry()
            .get_canister_principal(canister_type)
            .ok_or(UpgraderError::CanisterNotFound)?;

        let arg = canister_args.encode_args()?;

        let arg = InstallCodeArgument {
            mode: CanisterInstallMode::Reinstall,
            canister_id: principal,
            wasm_module: wasm,
            arg,
        };
        Management::install_code(arg)
            .await
            .map_err(|e| UpgraderError::CanisterReinstallFailed(e.to_string()))?;

        // Update the registry
        state.borrow_mut().mut_registry().update_canister_status(
            principal,
            CanisterStatus::Reinstalled,
            Some(wasm_hash),
        );

        Ok(())
    }

    /// Deletes the canister with the provided canister type.
    ///
    /// # Arguments
    /// * `canister_type` - The type of the canister to be deleted.
    ///
    /// # Returns
    /// A result indicating whether the deletion was successful.
    #[update]
    pub async fn delete(&self, canister_type: CanisterType) -> Result {
        let state = get_state();
        let principal = state
            .borrow()
            .registry()
            .get_canister_principal(canister_type)
            .ok_or(UpgraderError::CanisterNotFound)?;

        Management::stop_canister(principal).await?;

        Management::uninstall_code(principal).await?;

        Management::delete_canister(principal).await?;

        state
            .borrow_mut()
            .mut_registry()
            .remove_canister(&principal);

        Ok(())
    }

    /// Returns the CanisterType for the given canister principal, if it exists in the registry.
    ///
    /// # Arguments
    /// * `principal` - The principal of the canister to get the type for.
    ///
    /// # Returns
    /// The CanisterType for the given principal, if it exists in the registry. Otherwise, `None`.
    #[query]
    pub fn get_canister_info(&self, principal: Principal) -> Option<CanisterType> {
        let state = get_state();
        let state = state.borrow();

        state
            .registry()
            .get_canister_info(&principal)
            .map(|info| info.canister_type.clone())
    }

    /// Returns the principal of the canister with the given canister type, if it exists in the registry.
    ///
    /// # Arguments
    /// * `canister_type` - The type of the canister to get the principal for.
    ///
    /// # Returns
    /// The principal of the canister with the given type, if it exists in the registry. Otherwise, `None`.
    #[query]
    pub fn get_canister_principal(&self, canister_type: CanisterType) -> Option<Principal> {
        let state = get_state();
        let state = state.borrow();

        state.registry().get_canister_principal(canister_type)
    }

    #[query]
    pub fn get_all_canister_info(&self) -> Vec<(Principal, CanisterInfo)> {
        let state = get_state();
        let state = state.borrow();

        state.registry().get_all_canisters()
    }

    #[update]
    pub async fn add_implementation_to_bft(
        &self,
        bridge_address: H160,
        evm: Principal,
        bytecode: String,
    ) -> Result<H256> {
        let client = IcCanisterClient::new(evm);
        let evm_canister = EvmCanisterClient::new(client);
        let signer = get_state().borrow().signer.get_transaction_signer();

        let hashed_bytecode = keccak256(&bytecode);
        let encoded_data = minter_contract_utils::bft_bridge_api::ADD_IMPLEMENTATION
            .encode_input(&[Token::Bytes(hashed_bytecode.into())])
            .map_err(|e| UpgraderError::InternalError(e.to_string()))?;

        let from = signer
            .get_address()
            .await
            .map_err(|e| UpgraderError::TransactionSignerError(e.to_string()))?;

        let nonce = evm_canister
            .account_basic(from.clone())
            .await
            .map_err(|e| UpgraderError::InternalError(e.to_string()))?
            .nonce;

        pub const DEFAULT_TX_GAS_LIMIT: u64 = 3_000_000;

        let mut tx = ethers_core::types::Transaction {
            nonce: nonce.into(),
            gas: DEFAULT_TX_GAS_LIMIT.into(),
            from: from.into(),
            to: Some(bridge_address.into()),
            input: encoded_data.into(),
            ..Default::default()
        };

        let signature = signer
            .sign_transaction(&(&tx).into())
            .await
            .map_err(|e| UpgraderError::TransactionSignerError(e.to_string()))?;

        tx.r = signature.r.0;
        tx.s = signature.s.0;
        tx.v = signature.v.0;
        tx.hash = tx.hash();

        let tx_id = evm_canister
            .send_raw_transaction(tx.into())
            .await
            .map_err(|e| UpgraderError::InternalError(e.to_string()))?
            .map_err(|e| UpgraderError::InternalError(e.to_string()))?;

        Ok(tx_id)
    }

    /// Helper function to deploy a canister with the provided principal, WASM
    /// code, and arguments.
    async fn deploy_canister(
        CanisterIdRecord { canister_id }: CanisterIdRecord,
        wasm: Vec<u8>,
        arg: Vec<u8>,
    ) -> Result {
        let arg = InstallCodeArgument {
            mode: CanisterInstallMode::Install,
            canister_id,
            wasm_module: wasm,
            arg,
        };
        Management::install_code(arg)
            .await
            .map_err(|e| UpgraderError::CanisterInstallationFailed(e.to_string()))?;

        let status = Management::canister_status(canister_id).await?;

        match status.status {
            CanisterStatusType::Running => Ok(()),
            _ => Err(UpgraderError::CanisterNotRunning(canister_id)),
        }
    }

    /// Validates the provided WASM code for the given canister type.
    pub fn validate(&self, canister_type: &CanisterType, wasm: Vec<u8>) -> Result {
        let marker = canister_type.marker();

        let marker_is_valid = wasm
            .windows(marker.len())
            .any(|window| window == marker.as_bytes());

        if !marker_is_valid {
            return Err(UpgraderError::ValidationError);
        }

        Ok(())
    }

    pub fn idl() -> Idl {
        ic_canister::generate_idl!()
    }
}

impl Metrics for CanisterFactory {
    fn metrics(&self) -> Rc<RefCell<MetricsStorage>> {
        use ic_storage::IcStorage;
        MetricsStorage::get()
    }
}

thread_local! {
    pub(crate) static STATE: Rc<RefCell<State>> = Rc::default();
}

pub(crate) fn get_state() -> Rc<RefCell<State>> {
    STATE.with(|state| state.clone())
}

/// inspect function to check whether provided principal is owner
fn inspect_check_is_owner(principal: Principal, state: &State) -> Result {
    let owner = state.config.get_owner();

    if owner != principal {
        return Err(UpgraderError::Unauthorized { caller: principal });
    }

    Ok(())
}

/// inspect function to check whether the provided principal is anonymous
fn check_anonymous_principal(principal: Principal) -> Result {
    if principal == Principal::anonymous() {
        return Err(UpgraderError::AnonymousPrincipal);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use ic_canister::canister_call;
    use ic_exports::ic_kit::MockContext;

    use super::*;

    #[test]
    fn test_canister_type_marker() {
        let canister_type = CanisterType::ERC20;
        let marker = canister_type.marker();

        assert_eq!(marker, "ERC20");
    }

    #[test]
    fn test_canister_wasm_validation() {
        MockContext::new().inject();

        let canister_type = CanisterType::ERC20;
        let wasm = b"ERC20_BRIDGE_CANISTER".to_vec();

        let factory = CanisterFactory::from_principal(Principal::anonymous());

        let result = factory.validate(&canister_type, wasm);

        assert!(result.is_ok());
    }

    #[test]
    fn test_canister_wasm_validation_invalid() {
        MockContext::new().inject();

        let canister_type = CanisterType::ERC20;
        let wasm = b"INVALID_WASM".to_vec();

        let factory = CanisterFactory::from_principal(Principal::anonymous());

        let result = factory.validate(&canister_type, wasm);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_owner() {
        let context = MockContext::new().inject();
        let owner = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();

        let init = UpgraderInitData {
            owner,
            signing_strategy: SigningStrategy::Local {
                private_key: [5; 32],
            },
        };

        let canister_principal = Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").unwrap();

        let factory = CanisterFactory::from_principal(canister_principal);

        context.update_id(canister_principal);

        canister_call!(factory.init(init), ()).await.unwrap();

        let result = factory.get_owner();

        assert_eq!(result, owner);
    }
}
