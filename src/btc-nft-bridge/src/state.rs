use std::path::PathBuf;

use bitcoin::bip32::ChainCode;
use bitcoin::{Network, PublicKey};
use candid::{CandidType, Principal};
use did::H160;
use eth_signer::sign_strategy::{SigningStrategy, TxSigner};
use ic_exports::ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;
use ic_exports::ic_cdk::api::management_canister::ecdsa::{
    EcdsaCurve, EcdsaKeyId, EcdsaPublicKeyResponse,
};
use ic_log::{init_log, LogSettings};
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{StableCell, VirtualMemory};
use inscriber::interface::ecdsa_api::{EcdsaSigner, MasterKey};
use minter_contract_utils::evm_bridge::{EvmInfo, EvmParams};
use minter_contract_utils::evm_link::EvmLink;
use serde::Deserialize;

use crate::constant::{MAINNET_CHAIN_ID, REGTEST_CHAIN_ID, TESTNET_CHAIN_ID};
use crate::interface::store::{BurnRequestStore, MintOrdersStore, NftStore};
use crate::memory::{MEMORY_MANAGER, SIGNER_MEMORY_ID};

type SignerStorage = StableCell<TxSigner, VirtualMemory<DefaultMemoryImpl>>;

pub struct State {
    config: BtcNftBridgeConfig,
    bridge_config: NftBridgeConfig,
    signer: SignerStorage,
    mint_orders: MintOrdersStore,
    burn_requests: BurnRequestStore,
    master_key: Option<MasterKey>,
    inscriptions: NftStore,
    evm_params: Option<EvmParams>,
}

#[derive(Debug, CandidType, Deserialize)]
pub struct BtcNftBridgeConfig {
    pub network: BitcoinNetwork,
    pub evm_link: EvmLink,
    pub signing_strategy: SigningStrategy,
    pub admin: Principal,
    pub ord_url: String,
    pub logger: LogSettings,
}

impl Default for BtcNftBridgeConfig {
    fn default() -> Self {
        Self {
            network: BitcoinNetwork::Regtest,
            evm_link: EvmLink::default(),
            signing_strategy: SigningStrategy::Local {
                private_key: [0; 32],
            },
            admin: Principal::management_canister(),
            ord_url: String::new(),
            logger: LogSettings::default(),
        }
    }
}

impl BtcNftBridgeConfig {
    fn validate_indexer_url(&self) -> Result<(), String> {
        if !self.ord_url.starts_with("https") {
            return Err(format!(
                "Indexer URL must be HTTPS. Given: {}",
                self.ord_url
            ));
        }

        Ok(())
    }
}

#[derive(Default, Debug, CandidType, Deserialize)]
pub struct NftBridgeConfig {
    pub erc721_chain_id: u32,
    pub bridge_address: H160,
    pub token_address: H160,
    pub token_name: [u8; 32],
    pub token_symbol: [u8; 16],
}

#[derive(Debug, CandidType, Clone, Deserialize, PartialEq)]
pub struct RpcConfig {
    pub bitcoin_rpc_url: Option<String>,
    pub bitcoin_rpc_username: Option<String>,
    pub bitcoin_rpc_password: Option<String>,
    pub bitcoin_data_dir: Option<PathBuf>,
    pub cookie_file: Option<PathBuf>,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            bitcoin_rpc_url: Some("http://127.0.0.1:18443".to_string()),
            bitcoin_rpc_username: Some("user".to_string()),
            bitcoin_rpc_password: Some("pass".to_string()),
            bitcoin_data_dir: None,
            cookie_file: None,
        }
    }
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
            bridge_config: Default::default(),
            signer,
            mint_orders: Default::default(),
            burn_requests: Default::default(),
            master_key: None,
            inscriptions: NftStore::default(),
            evm_params: None,
        }
    }
}

impl State {
    pub fn configure(&mut self, config: BtcNftBridgeConfig) {
        #[cfg(target_family = "wasm")]
        ic_crypto_getrandom_for_wasm::register_custom_getrandom();

        if let Err(err) = config.validate_indexer_url() {
            panic!("Invalid configuration: {err}");
        }

        let signer = config
            .signing_strategy
            .clone()
            .make_signer(0)
            .expect("Failed to create signer");
        let stable = SignerStorage::new(MEMORY_MANAGER.with(|mm| mm.get(SIGNER_MEMORY_ID)), signer)
            .expect("failed to init signer in stable memory");
        self.signer = stable;

        init_log(&config.logger).expect("failed to init logger");

        self.config = config;
    }

    pub fn configure_bft(&mut self, bft_config: NftBridgeConfig) {
        self.bridge_config = bft_config;
    }

    pub fn has_nft(&self, reveal_txid: &str) -> bool {
        self.inscriptions.has_inscription(reveal_txid)
    }

    pub fn ord_url(&self) -> String {
        self.config
            .ord_url
            .strip_suffix('/')
            .unwrap_or_else(|| &self.config.ord_url)
            .to_string()
    }

    pub fn erc721_chain_id(&self) -> u32 {
        self.bridge_config.erc721_chain_id
    }

    pub fn nft_token_address(&self) -> H160 {
        self.bridge_config.token_address.clone()
    }

    pub fn btc_chain_id(&self) -> u32 {
        match self.config.network {
            BitcoinNetwork::Mainnet => MAINNET_CHAIN_ID,
            BitcoinNetwork::Testnet => TESTNET_CHAIN_ID,
            BitcoinNetwork::Regtest => REGTEST_CHAIN_ID,
        }
    }

    pub fn btc_network(&self) -> Network {
        match self.config.network {
            BitcoinNetwork::Mainnet => Network::Bitcoin,
            BitcoinNetwork::Testnet => Network::Testnet,
            BitcoinNetwork::Regtest => Network::Regtest,
        }
    }

    pub fn ic_btc_network(&self) -> BitcoinNetwork {
        self.config.network
    }

    pub fn signer(&self) -> &SignerStorage {
        &self.signer
    }

    /// Updates the ecdsa signing configuration with the given master key information.
    ///
    /// This configuration is used to derive public keys for different user addresses, so this
    /// configuration must be set before any of the transactions can be processed.
    pub fn configure_ecdsa(&mut self, master_key: EcdsaPublicKeyResponse) {
        let chain_code: &[u8] = &master_key.chain_code;
        self.master_key = Some(MasterKey {
            public_key: PublicKey::from_slice(&master_key.public_key)
                .expect("invalid public key slice"),
            chain_code: ChainCode::try_from(chain_code).expect("invalid chain code slice"),
            key_id: self.ecdsa_key_id(),
        });
    }

    pub fn ecdsa_key_id(&self) -> EcdsaKeyId {
        let key_name = match &self.config.signing_strategy {
            SigningStrategy::Local { .. } => "none".to_string(),
            SigningStrategy::ManagementCanister { key_id } => key_id.to_string(),
        };

        EcdsaKeyId {
            curve: EcdsaCurve::Secp256k1,
            name: key_name,
        }
    }

    pub fn ecdsa_signer(&self) -> EcdsaSigner {
        EcdsaSigner::new(
            self.config.signing_strategy.clone(),
            self.master_key.clone(),
            self.btc_network(),
        )
    }

    pub fn mint_orders_mut(&mut self) -> &mut MintOrdersStore {
        &mut self.mint_orders
    }

    pub fn burn_requests_mut(&mut self) -> &mut BurnRequestStore {
        &mut self.burn_requests
    }

    pub fn inscriptions_mut(&mut self) -> &mut NftStore {
        &mut self.inscriptions
    }

    pub fn get_evm_info(&self) -> EvmInfo {
        EvmInfo {
            link: self.config.evm_link.clone(),
            bridge_contract: self.bridge_config.bridge_address.clone(),
            params: self.evm_params.clone(),
        }
    }

    pub fn get_evm_params(&self) -> &Option<EvmParams> {
        &self.evm_params
    }

    pub fn token_name(&self) -> [u8; 32] {
        self.bridge_config.token_name
    }

    pub fn token_symbol(&self) -> [u8; 16] {
        self.bridge_config.token_symbol
    }

    pub fn update_evm_params(&mut self, f: impl FnOnce(&mut Option<EvmParams>)) {
        f(&mut self.evm_params)
    }

    pub fn admin(&self) -> Principal {
        self.config.admin
    }

    pub fn check_admin(&self, caller: Principal) {
        if caller != self.admin() {
            panic!("access denied");
        }
    }
}
