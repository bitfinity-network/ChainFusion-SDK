use candid::CandidType;
use ethereum_json_rpc_client::{Client, EthGetLogsParams, EthJsonRpcClient};
use ethers_core::abi::{
    Constructor, Event, EventParam, Function, Param, ParamType, RawLog, StateMutability, Token,
};
use ethers_core::types::{BlockNumber as EthBlockNumber, Log, Transaction, H160, U256};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

pub static CONSTRUCTOR: Lazy<Constructor> = Lazy::new(|| Constructor {
    inputs: vec![Param {
        name: "minterAddress".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
});

#[allow(deprecated)] // need to initialize `constant` field
pub static MINTER_CANISTER_ADDRESS: Lazy<Function> = Lazy::new(|| Function {
    name: "minterCanisterAddress".into(),
    inputs: vec![],
    outputs: vec![Param {
        name: "".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::View,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static BURN: Lazy<Function> = Lazy::new(|| Function {
    name: "burn".into(),
    inputs: vec![
        Param {
            name: "amount".into(),
            kind: ParamType::Uint(256),
            internal_type: None,
        },
        Param {
            name: "fromERC721".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
        Param {
            name: "recipientID".into(),
            kind: ParamType::Bytes,
            internal_type: None,
        },
    ],
    outputs: vec![Param {
        name: "".into(),
        kind: ParamType::Uint(32),
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static MINT: Lazy<Function> = Lazy::new(|| Function {
    name: "mint".into(),
    inputs: vec![Param {
        name: "encodedOrder".into(),
        kind: ParamType::Bytes,
        internal_type: None,
    }],
    outputs: vec![],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static DEPLOY_WRAPPED_TOKEN: Lazy<Function> = Lazy::new(|| Function {
    name: "deployERC721".into(),
    inputs: vec![
        Param {
            name: "name".into(),
            kind: ParamType::String,
            internal_type: None,
        },
        Param {
            name: "symbol".into(),
            kind: ParamType::String,
            internal_type: None,
        },
        Param {
            name: "baseTokenID".into(),
            kind: ParamType::FixedBytes(32),
            internal_type: None,
        },
    ],
    outputs: vec![Param {
        name: "".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

pub static BURN_EVENT: Lazy<Event> = Lazy::new(|| Event {
    name: "BurnTokenEvent".into(),
    inputs: vec![
        EventParam {
            name: "sender".into(),
            kind: ParamType::Address,
            indexed: false,
        },
        EventParam {
            name: "fromERC721".into(),
            kind: ParamType::Address,
            indexed: false,
        },
        EventParam {
            name: "recipientID".into(),
            kind: ParamType::Bytes,
            indexed: false,
        },
        EventParam {
            name: "toToken".into(),
            kind: ParamType::FixedBytes(32),
            indexed: false,
        },
        EventParam {
            name: "operationID".into(),
            kind: ParamType::Uint(32),
            indexed: false,
        },
        EventParam {
            name: "name".into(),
            kind: ParamType::FixedBytes(32),
            indexed: false,
        },
        EventParam {
            name: "symbol".into(),
            kind: ParamType::FixedBytes(16),
            indexed: false,
        },
        EventParam {
            name: "tokenURI".into(),
            kind: ParamType::String,
            indexed: false,
        },
    ],
    anonymous: false,
});

/// Emited when token is burnt or minted by ERC721Bridge.
#[derive(Debug, Clone, CandidType, Serialize, Deserialize)]
pub enum BridgeEvent {
    Burnt(BurnEventData),
    Minted(MintedEventData),
}

impl BridgeEvent {
    pub async fn collect_logs(
        evm_client: &EthJsonRpcClient<impl Client>,
        from_block: EthBlockNumber,
        to_block: EthBlockNumber,
        bridge_contract: H160,
    ) -> Result<Vec<Log>, anyhow::Error> {
        let params = EthGetLogsParams {
            address: Some(vec![bridge_contract]),
            from_block,
            to_block,
            topics: Some(vec![vec![BURN_EVENT.signature(), MINTED_EVENT.signature()]]),
        };

        evm_client.get_logs(params).await
    }

    pub fn from_log(log: Log) -> Result<Self, ethers_core::abi::Error> {
        let raw_log = RawLog {
            topics: log.topics,
            data: log.data.to_vec(),
        };

        Self::try_from(raw_log)
    }
}

impl TryFrom<RawLog> for BridgeEvent {
    type Error = ethers_core::abi::Error;

    fn try_from(log: RawLog) -> Result<Self, Self::Error> {
        BurnEventData::try_from(log.clone())
            .map(Self::Burnt)
            .or_else(|_| MintedEventData::try_from(log).map(Self::Minted))
    }
}

/// Emited when token is burnt by ERC721Bridge.
#[derive(Debug, Default, Clone, CandidType, Serialize, Deserialize)]
pub struct BurnEventData {
    pub sender: did::H160,
    pub from_erc721: did::H160,
    pub recipient_id: Vec<u8>,
    pub to_token: Vec<u8>,
    pub operation_id: u32,
    pub name: Vec<u8>,
    pub symbol: Vec<u8>,
    pub nft_id: String,
}

/// Builds `BurntEventData` from tokens.
#[derive(Debug, Default)]
struct BurnEventDataBuilder {
    pub sender: Option<H160>,
    pub from_erc721: Option<H160>,
    pub recipient_id: Option<Vec<u8>>,
    pub to_token: Option<Vec<u8>>,
    pub operation_id: Option<u32>,
    pub name: Option<Vec<u8>>,
    pub symbol: Option<Vec<u8>>,
    pub nft_id: Option<String>,
}

impl BurnEventDataBuilder {
    /// Builds `BurntEventData` from tokens.
    /// All fields are required.
    fn build(self) -> Result<BurnEventData, ethers_core::abi::Error> {
        fn not_found(field: &str) -> impl FnOnce() -> ethers_core::abi::Error {
            let msg = format!("missing event field `{}`", field);
            move || ethers_core::abi::Error::Other(msg.into())
        }

        Ok(BurnEventData {
            sender: self.sender.ok_or_else(not_found("sender"))?.into(),
            from_erc721: self.from_erc721.ok_or_else(not_found("fromerc721"))?.into(),
            recipient_id: self.recipient_id.ok_or_else(not_found("recipientID"))?,
            to_token: self.to_token.ok_or_else(not_found("toToken"))?,
            operation_id: self.operation_id.ok_or_else(not_found("operationID"))?,
            name: self.name.ok_or_else(not_found("name"))?,
            symbol: self.symbol.ok_or_else(not_found("symbol"))?,
            nft_id: self.nft_id.ok_or_else(not_found("nftID"))?,
        })
    }

    fn with_field_from_token(mut self, name: &str, value: Token) -> Self {
        match name {
            "sender" => self.sender = value.into_address(),
            "fromerc721" => self.from_erc721 = value.into_address(),
            "recipientID" => self.recipient_id = value.into_bytes(),
            "toToken" => self.to_token = value.into_fixed_bytes(),
            "operationID" => self.operation_id = value.into_uint().map(|v| v.as_u32()),
            "name" => self.name = value.into_fixed_bytes(),
            "symbol" => self.symbol = value.into_fixed_bytes(),
            "nftID" => self.nft_id = value.into_string(),
            _ => {}
        };
        self
    }
}

impl TryFrom<RawLog> for BurnEventData {
    type Error = ethers_core::abi::Error;

    fn try_from(log: RawLog) -> Result<Self, Self::Error> {
        let parsed = BURN_EVENT.parse_log(log)?;

        let mut data_builder = BurnEventDataBuilder::default();

        for param in parsed.params {
            data_builder = data_builder.with_field_from_token(&param.name, param.value);
        }

        data_builder.build()
    }
}

pub static MINTED_EVENT: Lazy<Event> = Lazy::new(|| Event {
    name: "MintTokenEvent".into(),
    inputs: vec![
        EventParam {
            name: "fromToken".into(),
            kind: ParamType::FixedBytes(32),
            indexed: false,
        },
        EventParam {
            name: "senderID".into(),
            kind: ParamType::FixedBytes(32),
            indexed: false,
        },
        EventParam {
            name: "toERC721".into(),
            kind: ParamType::Address,
            indexed: false,
        },
        EventParam {
            name: "recipient".into(),
            kind: ParamType::Address,
            indexed: false,
        },
        EventParam {
            name: "tokenId".into(),
            kind: ParamType::Uint(32),
            indexed: false,
        },
        EventParam {
            name: "nonce".into(),
            kind: ParamType::Uint(32),
            indexed: false,
        },
    ],
    anonymous: false,
});

/// Event emitted when token is minted by ERC721Bridge.
#[derive(Debug, Default, Clone, CandidType, Serialize, Deserialize)]
pub struct MintedEventData {
    pub from_token: Vec<u8>,
    pub sender_id: Vec<u8>,
    pub to_erc721: did::H160,
    pub recipient: did::H160,
    pub nonce: u32,
}

/// Builds `MintedEventData` from tokens.
#[derive(Debug, Default)]
struct MintedEventDataBuilder {
    pub from_token: Option<Vec<u8>>,
    pub sender_id: Option<Vec<u8>>,
    pub to_erc721: Option<H160>,
    pub recipient: Option<H160>,
    pub nonce: Option<u32>,
}

impl MintedEventDataBuilder {
    /// Builds `MintedEventData` from tokens.
    /// All fields are required.
    fn build(self) -> Result<MintedEventData, ethers_core::abi::Error> {
        fn not_found(field: &str) -> impl FnOnce() -> ethers_core::abi::Error {
            let msg = format!("missing event field `{}`", field);
            move || ethers_core::abi::Error::Other(msg.into())
        }

        Ok(MintedEventData {
            from_token: self.from_token.ok_or_else(not_found("fromToken"))?,
            sender_id: self.sender_id.ok_or_else(not_found("senderID"))?,
            to_erc721: self.to_erc721.ok_or_else(not_found("toERC721"))?.into(),
            recipient: self.recipient.ok_or_else(not_found("recipient"))?.into(),
            nonce: self.nonce.ok_or_else(not_found("nonce"))?,
        })
    }

    fn with_field_from_token(mut self, name: &str, value: Token) -> Self {
        match name {
            "fromToken" => self.from_token = value.into_fixed_bytes(),
            "senderID" => self.sender_id = value.into_fixed_bytes(),
            "toERC721" => self.to_erc721 = value.into_address().map(Into::into),
            "recipient" => self.recipient = value.into_address().map(Into::into),
            "nonce" => self.nonce = value.into_uint().map(|v| v.as_u32()),
            _ => {}
        };
        self
    }
}

impl TryFrom<RawLog> for MintedEventData {
    type Error = ethers_core::abi::Error;

    fn try_from(log: RawLog) -> Result<Self, Self::Error> {
        let parsed = MINTED_EVENT.parse_log(log)?;

        let mut data_builder = MintedEventDataBuilder::default();

        for param in parsed.params {
            data_builder = data_builder.with_field_from_token(&param.name, param.value);
        }

        data_builder.build()
    }
}

#[allow(deprecated)] // need to initialize `constant` field
pub static GET_WRAPPED_TOKEN: Lazy<Function> = Lazy::new(|| Function {
    name: "getWrappedToken".into(),
    inputs: vec![Param {
        name: "baseTokenID".into(),
        kind: ParamType::FixedBytes(32),
        internal_type: None,
    }],
    outputs: vec![Param {
        name: "".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::View,
});

pub fn mint_transaction(
    sender: H160,
    bridge: H160,
    nonce: U256,
    gas_price: U256,
    mint_order_data: Vec<u8>,
    chain_id: u32,
) -> Transaction {
    let data = MINT
        .encode_input(&[Token::Bytes(mint_order_data)])
        .expect("mint order encoding should pass");

    pub const DEFAULT_TX_GAS_LIMIT: u64 = 3_000_000;
    ethers_core::types::Transaction {
        from: sender,
        to: bridge.into(),
        nonce,
        value: U256::zero(),
        gas: DEFAULT_TX_GAS_LIMIT.into(),
        gas_price: Some(gas_price),
        input: data.into(),
        chain_id: Some(chain_id.into()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use did::{H160, H256};
    use ethers_core::abi::{Bytes, RawLog, Token};
    use ethers_core::utils::hex::traits::FromHex;

    use super::{BurnEventData, BurnEventDataBuilder, MintedEventData, MintedEventDataBuilder};

    #[test]
    fn minted_event_data_builder_test() {
        let amount = 42.into();
        let from_token = vec![1; 32];
        let sender_id = vec![2; 32];
        let to_erc721 = H160::from_slice(&[3; 20]);
        let recipient = H160::from_slice(&[4; 20]);
        let nonce = 42u64.into();

        let event = MintedEventDataBuilder::default()
            .with_field_from_token("amount", Token::Uint(amount))
            .with_field_from_token("fromToken", Token::FixedBytes(from_token.clone()))
            .with_field_from_token("senderID", Token::FixedBytes(sender_id.clone()))
            .with_field_from_token("toerc721", Token::Address(to_erc721.0))
            .with_field_from_token("recipient", Token::Address(recipient.0))
            .with_field_from_token("nonce", Token::Uint(nonce))
            .build()
            .unwrap();

        assert_eq!(event.from_token, from_token);
        assert_eq!(event.sender_id, sender_id);
        assert_eq!(event.to_erc721, to_erc721);
        assert_eq!(event.recipient, recipient);
        assert_eq!(event.nonce, nonce.as_u32());
    }

    #[test]
    fn burnt_event_data_builder_test() {
        let sender = H160::from_slice(&[3; 20]);
        let from_erc721 = H160::from_slice(&[3; 20]);
        let recipient_id = vec![2; 32];
        let to_token = vec![3; 32];
        let nft_id = "66bf2c7be3b0de6916ce8d29465ca7d7c6e27bd57238c25721c101fac34f39cfi0";
        let operation_id = 24.into();
        let name = vec![4; 32];
        let symbol = vec![5; 32];

        let event = BurnEventDataBuilder::default()
            .with_field_from_token("sender", Token::Address(sender.0))
            .with_field_from_token("fromerc721", Token::Address(from_erc721.0))
            .with_field_from_token("recipientID", Token::Bytes(recipient_id.clone()))
            .with_field_from_token("toToken", Token::FixedBytes(to_token.clone()))
            .with_field_from_token("operationID", Token::Uint(operation_id))
            .with_field_from_token("name", Token::FixedBytes(name.clone()))
            .with_field_from_token(
                "nftID",
                Token::String(
                    "66bf2c7be3b0de6916ce8d29465ca7d7c6e27bd57238c25721c101fac34f39cfi0"
                        .to_string(),
                ),
            )
            .with_field_from_token("symbol", Token::FixedBytes(symbol.clone()))
            .build()
            .unwrap();

        assert_eq!(event.sender, sender);
        assert_eq!(event.nft_id, nft_id);
        assert_eq!(event.from_erc721, from_erc721);
        assert_eq!(event.recipient_id, recipient_id);
        assert_eq!(event.to_token, to_token);
        assert_eq!(event.operation_id, operation_id.as_u32());
        assert_eq!(event.name, name);
        assert_eq!(event.symbol, symbol);
    }

    #[test]
    fn convert_raw_log_into_minted_event() {
        let raw = RawLog {
            topics: vec![H256::from_hex_str("0x4e37fc8684e0f7ad6a6c1178855450294a16b418314493bd7883699e6b3a0140").unwrap().0],
            data: Bytes::from_hex("0x00000000000000000000000000000000000000000000000000000000000003e80100056b29e76e9f3b04252ff67c2e623a34dd275f46e5b79f000000000000000100056b29a2d1f5f7d0d6e524a73194b76469eba08460ba4400000000000000000000000000000000000000119544f158a75a60beb83d3a44dd16100ad6d1e50000000000000000000000001e368dfb3f4a2d4e326d2111e6415ce54e7403250000000000000000000000000000000000000000000000000000000000000000").unwrap(),
        };

        let _event = MintedEventData::try_from(raw).unwrap();
    }

    #[test]
    fn convert_raw_log_into_burnt_event() {
        let raw = RawLog {
            topics: vec![H256::from_hex_str("0xfa3804fd5313cc219c6d3a833f7dbc2b1b48ac5edbae532006f1aa876a23eb79").unwrap().0],
            data: Bytes::from_hex("0x000000000000000000000000e41b09c6e9eaa79356b10f4181564b4bdb169d3500000000000000000000000000000000000000000000000000000000000003e80000000000000000000000002ea5d83d5a08d8556f726d3004a50aa8aa81c5c200000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000057617465726d656c6f6e0000000000000000000000000000000000000000000057544d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200100056b29dc8b8e5954eebac85b3145745362adfa50d8ad9e00000000000000").unwrap(),
        };

        let _event = BurnEventData::try_from(raw).unwrap();
    }
}
