use candid::{CandidType, Principal};
use ic_exports::ic_kit::RejectionCode;
use thiserror::Error;

pub type Result<T = (), E = UpgraderError> = std::result::Result<T, E>;

#[derive(Error, Debug, CandidType)]
pub enum UpgraderError {
    #[error("Unauthorized: caller {caller} is not the owner")]
    Unauthorized { caller: Principal },
    #[error("Canister not found, deploy the canister first")]
    CanisterNotFound,

    #[error("Canister installation failed: {0}")]
    CanisterInstallationFailed(String),

    #[error("Canister upgrade failed : {0}")]
    CanisterUpgradeFailed(String),

    #[error("Canister reinstallation failed : {0}")]
    CanisterReinstallFailed(String),

    #[error("Canister: {0} not running after operation")]
    CanisterNotRunning(Principal),

    #[error("Management canister error: Rejection Code :{0} Error: {1}")]
    ManagementCanisterError(String, String),

    #[error("Candid error: {0}")]
    CandidError(String),

    #[error("Validation error: the provided canister wasm module is invalid")]
    ValidationError,

    #[error("Anonymous call not allowed")]
    AnonymousPrincipal,

    #[error("Transaction signer error: {0}")]
    TransactionSignerError(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl From<(RejectionCode, String)> for UpgraderError {
    fn from((code, error): (RejectionCode, String)) -> Self {
        Self::ManagementCanisterError(format!("{:?}", code), error)
    }
}
