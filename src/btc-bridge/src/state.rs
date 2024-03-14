use crate::memory::{MEMORY_MANAGER, SIGNER_MEMORY_ID};
use crate::orders_store::OrdersStore;
use candid::{CandidType, Principal};
use did::H160;
use eth_signer::sign_strategy::{SigningStrategy, TxSigner};
use ic_exports::ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{StableCell, VirtualMemory};
use minter_contract_utils::evm_bridge::{EvmInfo, EvmParams};
use minter_contract_utils::evm_link::EvmLink;
use serde::Deserialize;

const MAINNET_CHAIN_ID: u32 = 0;
const TESTNET_CHAIN_ID: u32 = 1;
const REGTEST_CHAIN_ID: u32 = 2;

type SignerStorage = StableCell<TxSigner, VirtualMemory<DefaultMemoryImpl>>;

pub struct State {
    config: BtcBridgeConfig,
    bft_config: BftBridgeConfig,
    signer: SignerStorage,
    orders_store: OrdersStore,
    evm_params: Option<EvmParams>,
}

#[derive(Debug, CandidType, Deserialize)]
pub struct BtcBridgeConfig {
    pub ck_btc_minter: Principal,
    pub ck_btc_ledger: Principal,
    pub network: BitcoinNetwork,
    pub evm_link: EvmLink,
    pub signing_strategy: SigningStrategy,
    pub admin: Principal,
    pub ck_btc_ledger_fee: u64,
}

impl Default for BtcBridgeConfig {
    fn default() -> Self {
        Self {
            ck_btc_minter: Principal::anonymous(),
            ck_btc_ledger: Principal::anonymous(),
            network: BitcoinNetwork::Regtest,
            evm_link: EvmLink::default(),
            signing_strategy: SigningStrategy::Local {
                private_key: [0; 32],
            },
            admin: Principal::anonymous(),
            ck_btc_ledger_fee: 10,
        }
    }
}

#[derive(Default, Debug, CandidType, Deserialize)]
pub struct BftBridgeConfig {
    pub erc20_chain_id: u32,
    pub bridge_address: H160,
    pub token_address: H160,
    pub token_name: [u8; 32],
    pub token_symbol: [u8; 16],
    pub decimals: u8,
}

impl Default for State {
    fn default() -> Self {
        let default_signer = SigningStrategy::Local {
            private_key: [1; 32],
        }
        .make_signer(0)
        .expect("Failed to create default signer");

        let signer = SignerStorage::new(
            MEMORY_MANAGER.with(|mm| mm.get(SIGNER_MEMORY_ID)),
            default_signer,
        )
        .expect("failed to initialize transaction signer");

        Self {
            config: Default::default(),
            bft_config: Default::default(),
            signer,
            orders_store: Default::default(),
            evm_params: None,
        }
    }
}

impl State {
    pub fn configure(&mut self, config: BtcBridgeConfig) {
        let signer = config
            .signing_strategy
            .clone()
            .make_signer(0)
            .expect("Failed to create signer");
        let stable = SignerStorage::new(MEMORY_MANAGER.with(|mm| mm.get(SIGNER_MEMORY_ID)), signer)
            .expect("failed to init signer in stable memory");
        self.signer = stable;

        self.config = config;
    }

    pub fn configure_bft(&mut self, bft_config: BftBridgeConfig) {
        self.bft_config = bft_config;
    }

    pub fn ck_btc_minter(&self) -> Principal {
        self.config.ck_btc_minter
    }

    pub fn ck_btc_ledger(&self) -> Principal {
        self.config.ck_btc_ledger
    }

    pub fn erc20_chain_id(&self) -> u32 {
        self.bft_config.erc20_chain_id
    }

    pub fn btc_chain_id(&self) -> u32 {
        match self.config.network {
            BitcoinNetwork::Mainnet => MAINNET_CHAIN_ID,
            BitcoinNetwork::Testnet => TESTNET_CHAIN_ID,
            BitcoinNetwork::Regtest => REGTEST_CHAIN_ID,
        }
    }

    pub fn signer(&self) -> &SignerStorage {
        &self.signer
    }

    pub fn mint_orders(&self) -> &OrdersStore {
        &self.orders_store
    }

    pub fn mint_orders_mut(&mut self) -> &mut OrdersStore {
        &mut self.orders_store
    }

    pub fn get_evm_info(&self) -> EvmInfo {
        EvmInfo {
            link: self.config.evm_link.clone(),
            bridge_contract: self.bft_config.bridge_address.clone(),
            params: self.evm_params.clone(),
        }
    }

    pub fn get_evm_params(&self) -> &Option<EvmParams> {
        &self.evm_params
    }

    pub fn token_address(&self) -> &H160 {
        &self.bft_config.token_address
    }

    pub fn token_name(&self) -> [u8; 32] {
        self.bft_config.token_name
    }

    pub fn token_symbol(&self) -> [u8; 16] {
        self.bft_config.token_symbol
    }

    pub fn decimals(&self) -> u8 {
        self.bft_config.decimals
    }

    pub fn update_evm_params(&mut self, f: impl FnOnce(&mut Option<EvmParams>)) {
        f(&mut self.evm_params)
    }

    pub fn admin(&self) -> Principal {
        self.config.admin
    }

    pub fn ck_btc_ledger_fee(&self) -> u64 {
        self.config.ck_btc_ledger_fee
    }
}
