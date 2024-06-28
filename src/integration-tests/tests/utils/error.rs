use did::error::EvmError;
use ic_canister_client::CanisterClientError;
use ic_exports::icrc_types::icrc1::transfer::TransferError;
use ic_exports::icrc_types::icrc2::approve::ApproveError;
use ic_exports::pocket_ic::CallError;
use ic_test_utils::Error;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestError {
    #[error(transparent)]
    Evm(#[from] EvmError),

    #[error(transparent)]
    MinterCanister(#[from] minter_did::error::Error),

    #[error(transparent)]
    Candid(#[from] candid::Error),

    #[error(transparent)]
    CanisterClient(#[from] CanisterClientError),

    #[error(transparent)]
    TestUtils(#[from] Error),

    #[error(transparent)]
    Icrc(IcrcError),

    #[error("{0}")]
    Generic(String),
    #[error("Call error {0}")]
    CallError(String),
}

#[derive(Debug, Error)]
pub enum IcrcError {
    #[error("ICRC-2 transfer failed: {0:?}")]
    Transfer(TransferError),
    #[error("ICRC-2 approve failed: {0:?}")]
    Approve(ApproveError),
}

impl From<TransferError> for TestError {
    fn from(e: TransferError) -> Self {
        Self::Icrc(IcrcError::Transfer(e))
    }
}

impl From<ApproveError> for TestError {
    fn from(e: ApproveError) -> Self {
        Self::Icrc(IcrcError::Approve(e))
    }
}

impl From<CallError> for TestError {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Reject(reject) => Self::CallError(reject),
            CallError::UserError(user_err) => Self::CallError(user_err.to_string()),
        }
    }
}

impl From<String> for TestError {
    fn from(value: String) -> Self {
        Self::Generic(value)
    }
}

impl From<&str> for TestError {
    fn from(value: &str) -> Self {
        Self::Generic(value.into())
    }
}

pub type Result<T> = std::result::Result<T, TestError>;
