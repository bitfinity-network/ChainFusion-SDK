use candid::{CandidType, Encode};
use minter_did::init::InitData;
use serde::Deserialize;

use crate::error::{Result, UpgraderError};

#[derive(CandidType, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum CanisterType {
    ICRC,
    ERC20,
    BTC,
    RUNE,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum CanisterArgs {
    ICRC(InitData),
    ERC20(erc20_minter::state::Settings),
    BTC(btc_bridge::state::BtcBridgeConfig),
    RUNE(rune_bridge::state::RuneBridgeConfig),
}

impl CanisterType {
    pub fn marker(&self) -> &'static str {
        match self {
            CanisterType::ICRC => icrc2_minter::ICRC_CANISTER_MARKER,
            CanisterType::ERC20 => erc20_minter::ERC20_CANISTER_MARKER,
            CanisterType::BTC => btc_bridge::BTC_BRIDGE_CANISTER_MARKER,
            CanisterType::RUNE => rune_bridge::RUNE_BRIDGE_CANISTER_MARKER,
        }
    }
}

impl CanisterArgs {
    pub fn encode_args(&self) -> Result<Vec<u8>> {
        let args = match self {
            Self::ICRC(args) => {
                Encode!(args).map_err(|e| (UpgraderError::CandidError(e.to_string())))?
            }
            Self::ERC20(args) => {
                Encode!(args).map_err(|e| (UpgraderError::CandidError(e.to_string())))?
            }
            Self::BTC(args) => {
                Encode!(args).map_err(|e| (UpgraderError::CandidError(e.to_string())))?
            }
            Self::RUNE(args) => {
                Encode!(args).map_err(|e| (UpgraderError::CandidError(e.to_string())))?
            }
        };

        Ok(args)
    }

    pub fn _type(&self) -> CanisterType {
        match self {
            Self::ICRC(_) => CanisterType::ICRC,
            Self::ERC20(_) => CanisterType::ERC20,
            Self::BTC(_) => CanisterType::BTC,
            Self::RUNE(_) => CanisterType::RUNE,
        }
    }
}
