use std::str::FromStr;
use std::time::Duration;

use candid::{CandidType, Encode, IDLArgs, Principal, TypeEnv};
use clap::Parser;
use did::constant::EIP1559_INITIAL_BASE_FEE;
use did::{BlockNumber, Transaction, TransactionReceipt, H256, U256};
use eth_signer::transaction::{SigningMethod, TransactionBuilder};
use eth_signer::{Signer, Wallet};
use ethereum_types::H160;
use ethers_core::abi::Token;
use ethers_core::k256::ecdsa::SigningKey;
use evm_canister_client::EvmCanisterClient;
use ic_canister_client::IcAgentClient;
use minter_contract_utils::build_data::{
    BFT_BRIDGE_SMART_CONTRACT_CODE, FEE_CHARGE_SMART_CONTRACT_CODE, UUPS_PROXY_SMART_CONTRACT_CODE,
};
use minter_contract_utils::{bft_bridge_api, fee_charge_api, wrapped_token_api};
use minter_did::id256::Id256;
use minter_did::reason::Icrc2Burn;
use tokio::time::Instant;

// This identity is only used to make the calls non-anonymous. No actual checks depend on this
// identity.
const IDENTITY_PATH: &str = "src/bridge-tool/identity.pem";

/// Some operations with BFT bridge.
#[derive(Parser, Debug)]
#[clap(version = "0.1")]
enum CliCommand {
    /// Create bft bridge contract.
    DeployBftBridge(DeployBftArgs),
    /// Create wrapper token contract.
    CreateToken(CreateTokenArgs),
    /// Create a new ETH wallet and mint native tokens to it.
    CreateWallet(CreateWalletArgs),
    /// Burn wrapped BTC.
    BurnWrapped(BurnWrappedArgs),
    /// Return ETH wallet address.
    WalletAddress(WalletAddressArgs),
    /// Create bft bridge contract.
    DeployFeeCharge(DeployFeeChargeArgs),
    /// Returns expected contract address for the given parameters.
    ExpectedContractAddress(ExpectedContractAddress),
    /// Request ICRC2 deposit
    DepositIcrc(DepositIcrcArgs),
    /// Get wallet nonce
    GetNonce(GetNonceArgs),
}

#[derive(Debug, Parser)]
struct GetNonceArgs {
    /// Evm canister principal
    #[arg(long)]
    evm: Principal,
    #[arg(long)]

    /// PK of the EVM wallet to get nonce for
    wallet: Option<String>,

    /// IC host (uses local dfx deployment by default)
    #[arg(long)]
    ic_host: Option<String>,
}

#[derive(Debug, Parser)]
struct DepositIcrcArgs {
    /// Evm canister principal
    #[arg(long)]
    evm: Principal,

    /// EVM address of the BFT bridge
    #[arg(long)]
    bft_bridge: String,

    /// Amount to deposit
    #[arg(long)]
    amount: u128,

    /// Principal of the sender (from which account ICRC tokens are taken)
    #[arg(long)]
    sender: Principal,

    /// Principal of the ICRC2 token to be bridged
    #[arg(long)]
    token: Principal,

    /// IC host
    #[arg(long)]
    ic_host: Option<String>,

    /// Hex-encoded PK to use to sign transaction. If not set, a random wallet will be created.
    #[arg(long)]
    wallet: Option<String>,
}

#[derive(Debug, Parser)]
struct DeployBftArgs {
    /// ETH address of the minter
    #[arg(long)]
    minter_address: String,

    /// ETH address of the FeeCharge contract.
    #[arg(long)]
    fee_charge_address: Option<String>,

    /// IsWrappedSide
    #[arg(long, default_value_t = false)]
    is_wrapped_side: bool,

    /// Evm Principal
    #[arg(long)]
    evm: Principal,

    /// IC host
    #[arg(long)]
    ic_host: Option<String>,

    /// Hex-encoded PK to use to sign transaction. If not set, a random wallet will be created.
    #[arg(long)]
    wallet: Option<String>,

    /// Identity Path
    #[arg(long)]
    identity_path: Option<String>,
}

#[derive(Debug, Parser)]
struct DeployFeeChargeArgs {
    /// Principal of the EVM canister
    #[arg(long)]
    evm: Principal,

    #[arg(long)]
    identity_path: Option<String>,

    /// IC host
    #[arg(long)]
    ic_host: Option<String>,

    /// Hex-encoded PK to use to sign transaction.
    #[arg(long)]
    wallet: String,

    /// Nonce for the transaction. Should be fixed to know the contract address before the deploy.
    #[arg(long)]
    nonce: u64,

    /// Addresses of BftBridges, which should be able to charge fee.
    #[arg(long)]
    bridges: Vec<String>,
}

#[derive(Debug, Parser)]
struct ExpectedContractAddress {
    /// Hex-encoded PK of contract deployer.
    #[arg(long)]
    wallet: String,

    /// Nonce used in contract deployment transaction.
    #[arg(long)]
    nonce: u64,
}

#[derive(Debug, Parser)]
struct CreateTokenArgs {
    /// ETH address of the BFT bridge contract.
    #[arg(long)]
    bft_bridge_address: String,

    /// Name of the token to be created.
    #[arg(long)]
    token_name: String,

    /// ID of the source token.
    ///
    /// ID can be in one of the following forms:
    /// * raw hex value of `Id256`
    /// * principal of the token in case the token is hosted by IC
    /// * `BLOCK_ID:TX_INDEX` for runes
    #[arg(long)]
    token_id: String,

    /// Principal of the EVM canister.
    #[arg(long)]
    evm_canister: Principal,

    /// IC host
    #[arg(long)]
    ic_host: Option<String>,

    /// Identity Path
    #[arg(long)]
    identity_path: Option<String>,

    /// Hex-encoded PK to use to sign transaction. If not set, a random wallet will be created.
    #[arg(long)]
    wallet: Option<String>,
}

#[derive(Debug, Parser)]
struct CreateWalletArgs {
    /// Principal of the EVM canister.
    #[arg(long)]
    evm_canister: Principal,
}

#[derive(Debug, Parser)]
struct BurnWrappedArgs {
    /// Hex-encoded PK to use to sign transaction.
    #[arg(long)]
    wallet: String,

    /// Principal of the EVM canister.
    #[arg(long)]
    evm_canister: Principal,

    /// ETH address of the BFT bridge contract.
    #[arg(long)]
    bft_bridge: String,

    /// ETH address of the wrapper token contract.
    #[arg(long)]
    token_address: String,

    /// to Token ID.
    #[arg(long)]
    to_token_id: String,

    /// BTC address to transfer BTC to.
    #[arg(long)]
    address: String,

    /// Amount to transfer.
    #[arg(long)]
    amount: u128,
}

#[derive(Debug, Parser)]
struct WalletAddressArgs {
    /// Hex-encoded PK to use to sign transaction.
    #[arg(long)]
    wallet: String,

    /// If set, returns the address in candid form. Otherwise, in hex form.
    #[arg(long)]
    candid: bool,
}

#[tokio::main]
async fn main() {
    match CliCommand::parse() {
        CliCommand::DeployBftBridge(args) => deploy_bft_bridge(args).await,
        CliCommand::CreateToken(args) => create_token(args).await,
        CliCommand::CreateWallet(args) => create_wallet(args).await,
        CliCommand::BurnWrapped(args) => burn_wrapped(args).await,
        CliCommand::WalletAddress(args) => wallet_address(args),
        CliCommand::DeployFeeCharge(args) => deploy_fee_charge(args).await,
        CliCommand::ExpectedContractAddress(args) => expected_contract_address(args),
        CliCommand::DepositIcrc(args) => deposit_icrc(args).await,
        CliCommand::GetNonce(args) => get_nonce(args).await,
    }
}

async fn get_nonce(args: GetNonceArgs) {
    let host = args.ic_host.as_deref().unwrap_or("http://127.0.0.1:4943");

    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(args.evm, IDENTITY_PATH, host, None)
            .await
            .expect("Failed to create client"),
    );

    let wallet = get_wallet(&args.wallet, &client).await;
    let nonce = client
        .eth_get_transaction_count(wallet.address().into(), BlockNumber::Pending)
        .await
        .unwrap()
        .unwrap();

    println!("{nonce}");
}

async fn deposit_icrc(args: DepositIcrcArgs) {
    let bft_bridge = H160::from_slice(
        &hex::decode(args.bft_bridge.trim_start_matches("0x"))
            .expect("failed to parse bft bridge address"),
    );

    let host = args.ic_host.as_deref().unwrap_or("http://127.0.0.1:4943");

    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(args.evm, IDENTITY_PATH, host, None)
            .await
            .expect("Failed to create client"),
    );

    let wallet = get_wallet(&args.wallet, &client).await;
    let chain_id = client.eth_chain_id().await.expect("failed to get chain id");

    let data = Icrc2Burn {
        sender: args.sender,
        amount: args.amount.into(),
        icrc2_token_principal: args.token,
        from_subaccount: None,
        recipient_address: wallet.address().into(),
        approve_after_mint: None,
        fee_payer: None,
    };

    let input = bft_bridge_api::NOTIFY_MINTER
        .encode_input(&[
            Token::Uint(0u32.into()),
            Token::Bytes(Encode!(&data).unwrap()),
        ])
        .unwrap();

    let nonce = client
        .account_basic(wallet.address().into())
        .await
        .expect("Failed to get account info.")
        .nonce;
    let notify_minter_tx = TransactionBuilder {
        from: &wallet.address().into(),
        to: Some(bft_bridge.into()),
        nonce,
        value: 0u64.into(),
        gas: 5_000_000u64.into(),
        gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
        input,
        signature: SigningMethod::SigningKey(wallet.signer()),
        chain_id,
    }
    .calculate_hash_and_build()
    .expect("failed to sign the transaction");

    let hash = client
        .send_raw_transaction(notify_minter_tx)
        .await
        .expect("Failed to send raw transaction")
        .expect("Failed to execute deposit notification transaction");

    let receipt = wait_for_tx_success(&client, hash).await;

    eprintln!("Deposit notification sent");
    eprintln!("Transaction receipt: {receipt:?}");
}

async fn get_wallet<'a>(
    pk: &'a Option<String>,
    client: &'a EvmCanisterClient<IcAgentClient>,
) -> Wallet<'a, SigningKey> {
    match pk {
        Some(v) => Wallet::from_bytes(
            &hex::decode(v.trim_start_matches("0x")).expect("invalid hex string for wallet PK"),
        )
        .expect("invalid wallet PK value"),
        None => create_new_wallet(client).await,
    }
}

async fn create_new_wallet(client: &EvmCanisterClient<IcAgentClient>) -> Wallet<SigningKey> {
    let wallet = Wallet::new(&mut rand::thread_rng());
    eprintln!("Initialized new wallet: {:#x}", wallet.address());

    mint_tokens(client, &wallet).await;

    wallet
}

async fn mint_tokens(client: &EvmCanisterClient<IcAgentClient>, wallet: &Wallet<'_, SigningKey>) {
    let res = client
        .mint_native_tokens(wallet.address().into(), u128::MAX.into())
        .await
        .expect("Failed to send mint native tokens request")
        .expect("Mint native tokens request failed");

    wait_for_tx_success(client, res.0.clone()).await;
    eprintln!(
        "Minted {} ETH tokens to address {:#x}",
        u128::MAX,
        wallet.address()
    );
}

const MAX_TX_TIMEOUT_SEC: u64 = 6;

async fn wait_for_tx_success(
    client: &EvmCanisterClient<IcAgentClient>,
    tx_hash: H256,
) -> TransactionReceipt {
    let start = Instant::now();
    let timeout = Duration::from_secs(MAX_TX_TIMEOUT_SEC);
    while start.elapsed() < timeout {
        let receipt = client
            .eth_get_transaction_receipt(tx_hash.clone())
            .await
            .expect("Failed to request transaction receipt")
            .expect("Request for receipt failed");

        if let Some(receipt) = receipt {
            if receipt.status != Some(1u64.into()) {
                eprintln!("Transaction: {tx_hash}");
                eprintln!("Receipt: {receipt:?}");
                if let Some(output) = receipt.output {
                    let output = String::from_utf8_lossy(&output);
                    eprintln!("Output: {output}");
                }

                panic!("Transaction failed");
            } else {
                return receipt;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    panic!("Transaction {tx_hash} timed out");
}

fn _print_signed_tx(tx: Transaction) {
    let candid_bytes = candid::encode_args((&tx,)).expect("failed to serialize tx to Candid");
    let args = IDLArgs::from_bytes(&candid_bytes).expect("failed to deserialize Candid");
    // Without type annotation instead of field names numerical ids will be used in output
    let args = args
        .annotate_types(false, &TypeEnv::new(), &[Transaction::ty()])
        .unwrap();

    // Output the transaction in Candid text format
    println!("{args}");
}

async fn deploy_bft_bridge(args: DeployBftArgs) {
    let minter = H160::from_slice(
        &hex::decode(args.minter_address.trim_start_matches("0x"))
            .expect("failed to parse minter address"),
    );

    let identity = args.identity_path.as_deref().unwrap_or(IDENTITY_PATH);
    let host = args.ic_host.as_deref().unwrap_or("http://127.0.0.1:4943");
    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(args.evm, identity, host, None)
            .await
            .expect("failed to create evm client"),
    );

    let wallet = get_wallet(&args.wallet, &client).await;

    let chain_id = client.eth_chain_id().await.expect("failed to get chain id");

    let fee_charge = args
        .fee_charge_address
        .map(|address_str| {
            H160::from_slice(
                &hex::decode(address_str.trim_start_matches("0x"))
                    .expect("failed to parse minter address"),
            )
        })
        .unwrap_or_default();

    async fn deploy_contract(
        client: &EvmCanisterClient<IcAgentClient>,
        wallet: &Wallet<'_, SigningKey>,
        input: Vec<u8>,
        chain_id: u64,
    ) -> H160 {
        let nonce = client
            .eth_get_transaction_count(wallet.address().into(), BlockNumber::Pending)
            .await
            .unwrap()
            .unwrap();

        let tx = TransactionBuilder {
            from: &wallet.address().into(),
            to: None,
            nonce,
            value: 0u64.into(),
            gas: 5_000_000u64.into(),
            gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
            input,
            signature: SigningMethod::SigningKey(wallet.signer()),
            chain_id: chain_id as _,
        }
        .calculate_hash_and_build()
        .expect("Failed to sign the transaction");

        let hash = client
            .send_raw_transaction(tx)
            .await
            .expect("Failed to send raw transaction")
            .expect("Failed to execute contract deployment transaction");
        let receipt = wait_for_tx_success(client, hash).await;
        receipt
            .contract_address
            .expect("Receipt did not contain contract address")
            .into()
    }

    let bft_contract_input = bft_bridge_api::CONSTRUCTOR
        .encode_input(BFT_BRIDGE_SMART_CONTRACT_CODE.clone(), &[])
        .unwrap();

    let bft_contract_address =
        deploy_contract(&client, &wallet, bft_contract_input, chain_id).await;

    let initialize_data = bft_bridge_api::proxy::INITIALISER
        .encode_input(&[
            Token::Address(minter),
            Token::Address(fee_charge),
            Token::Bool(args.is_wrapped_side),
        ])
        .expect("failed to encode initialize data");

    let proxy_input = bft_bridge_api::proxy::CONSTRUCTOR
        .encode_input(
            UUPS_PROXY_SMART_CONTRACT_CODE.clone(),
            &[
                Token::Address(bft_contract_address),
                Token::Bytes(initialize_data),
            ],
        )
        .expect("failed to encode proxy constructor input");

    let bft_proxy_address = deploy_contract(&client, &wallet, proxy_input, chain_id).await;

    eprintln!("Created BFT Bridge contract");
    println!("Implementation address: {bft_contract_address:#x}");
    println!("Proxy address: {bft_proxy_address:#x}");
    println!("{bft_proxy_address:#x}");
}

async fn deploy_fee_charge(args: DeployFeeChargeArgs) {
    let identity = args.identity_path.as_deref().unwrap_or(IDENTITY_PATH);
    let host = args.ic_host.as_deref().unwrap_or("http://127.0.0.1:4943");
    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(args.evm, identity, host, None)
            .await
            .expect("failed to create evm client"),
    );

    let some_wallet = Some(args.wallet.clone());
    let wallet = get_wallet(&some_wallet, &client).await;
    let did_from: did::H160 = wallet.address().into();

    let addresses = args
        .bridges
        .iter()
        .map(|addr| {
            let addr = H160::from_slice(
                &hex::decode(addr.trim_start_matches("0x"))
                    .expect("failed to parse bridge address"),
            );
            Token::Address(addr)
        })
        .collect();
    let input = fee_charge_api::CONSTRUCTOR
        .encode_input(
            FEE_CHARGE_SMART_CONTRACT_CODE.clone(),
            &[Token::Array(addresses)],
        )
        .expect("failed to encode constructor input");

    let chain_id = client.eth_chain_id().await.expect("failed to get chain id");

    let create_contract_tx = TransactionBuilder {
        from: &did_from,
        input,
        nonce: args.nonce.into(),
        gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
        to: None,
        value: U256::zero(),
        gas: 4_000_000_u64.into(),
        signature: SigningMethod::SigningKey(wallet.signer()),
        chain_id,
    }
    .calculate_hash_and_build()
    .expect("failed to build transaction");

    let hash = client
        .send_raw_transaction(create_contract_tx)
        .await
        .expect("Failed to send raw transaction")
        .expect("Failed to execute crate BFT contract transaction");
    let receipt = wait_for_tx_success(&client, hash).await;
    let fee_charge_contract_address = receipt
        .contract_address
        .expect("Receipt did not contain contract address");

    eprintln!("Created FeeCharge contract");
    println!("{fee_charge_contract_address:#x}");
}

fn expected_contract_address(args: ExpectedContractAddress) {
    let wallet = Wallet::from_bytes(
        &hex::decode(args.wallet.trim_start_matches("0x"))
            .expect("invalid hex string for wallet PK"),
    )
    .expect("invalid wallet PK value");
    let deployer = wallet.address();
    let contract_address = ethers_core::utils::get_contract_address(deployer, args.nonce);
    println!("{contract_address:#x}");
}

async fn create_token(args: CreateTokenArgs) {
    let bft_bridge = H160::from_slice(
        &hex::decode(args.bft_bridge_address.trim_start_matches("0x"))
            .expect("failed to parse bft bridge address"),
    );

    let token_id = decode_token_id(&args.token_id)
        .unwrap_or_else(|| panic!("Invalid token id format: {}", args.token_id));

    let identity = args.identity_path.as_deref().unwrap_or(IDENTITY_PATH);
    let host = args.ic_host.as_deref().unwrap_or("http://127.0.0.1:4943");

    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(args.evm_canister, identity, host, None)
            .await
            .expect("Failed to create client"),
    );

    let wallet = get_wallet(&args.wallet, &client).await;
    let chain_id = client.eth_chain_id().await.expect("failed to get chain id");

    let input = bft_bridge_api::DEPLOY_WRAPPED_TOKEN
        .encode_input(&[
            Token::String(args.token_name.clone()),
            Token::String(args.token_name),
            Token::FixedBytes(token_id.0.to_vec()),
        ])
        .unwrap();

    let nonce = client
        .account_basic(wallet.address().into())
        .await
        .expect("Failed to get account info.")
        .nonce;
    let create_token_tx = TransactionBuilder {
        from: &wallet.address().into(),
        to: Some(bft_bridge.into()),
        nonce,
        value: 0u64.into(),
        gas: 5_000_000u64.into(),
        gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
        input,
        signature: SigningMethod::SigningKey(wallet.signer()),
        chain_id,
    }
    .calculate_hash_and_build()
    .expect("failed to sign the transaction");

    let hash = client
        .send_raw_transaction(create_token_tx)
        .await
        .expect("Failed to send raw transaction")
        .expect("Failed to execute crate token transaction");
    let receipt = wait_for_tx_success(&client, hash).await;

    let token_address = bft_bridge_api::DEPLOY_WRAPPED_TOKEN
        .decode_output(
            &receipt
                .output
                .expect("Receipt for token creation does not contain output"),
        )
        .expect("Failed to decode token creation output")[0]
        .clone()
        .into_address()
        .expect("Failed to decode token address");

    eprintln!("Created token contract");
    println!("{:#x}", token_address);
}

async fn create_wallet(args: CreateWalletArgs) {
    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(
            args.evm_canister,
            IDENTITY_PATH,
            "http://127.0.0.1:4943",
            None,
        )
        .await
        .expect("Failed to create client"),
    );

    let wallet = create_new_wallet(&client).await;

    eprint!("Wallet address, Candid style: blob \"");
    for num in wallet.address().0 {
        eprint!("\\{num:02x}");
    }
    for _ in 0..12 {
        eprint!("\\00");
    }
    eprintln!("\"");

    println!("0x{}", hex::encode(wallet.signer().to_bytes()));
}

fn wallet_address(args: WalletAddressArgs) {
    let wallet_pk = hex::decode(args.wallet.trim_start_matches("0x"))
        .expect("Failed to decode wallet pk from hex string");
    let wallet = Wallet::from_bytes(&wallet_pk).expect("Failed to create a wallet");

    if args.candid {
        print!("blob \"");
        for num in wallet.address().0 {
            print!("\\{num:02x}");
        }
        for _ in 0..12 {
            print!("\\00");
        }
        println!("\"");
    } else {
        println!("{:#x}", wallet.address());
    }
}

async fn burn_wrapped(args: BurnWrappedArgs) {
    let client = EvmCanisterClient::new(
        IcAgentClient::with_identity(
            args.evm_canister,
            IDENTITY_PATH,
            "http://127.0.0.1:4943",
            None,
        )
        .await
        .expect("Failed to create client"),
    );

    let wallet_addr = Some(args.wallet.clone());
    let wallet = get_wallet(&wallet_addr, &client).await;
    let chain_id = client.eth_chain_id().await.expect("failed to get chain id");

    let bft_bridge = H160::from_slice(
        &hex::decode(args.bft_bridge.trim_start_matches("0x"))
            .expect("failed to parse bft bridge address"),
    );

    let token = H160::from_slice(
        &hex::decode(args.token_address.trim_start_matches("0x"))
            .expect("failed to parse bft bridge address"),
    );

    let input = wrapped_token_api::ERC_20_BALANCE
        .encode_input(&[Token::Address(wallet.address())])
        .unwrap();
    let result = client
        .eth_call(
            Some(wallet.address().into()),
            Some(token.into()),
            None,
            5_000_000u64,
            Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
            Some(input.into()),
        )
        .await
        .expect("balance call failed")
        .expect("balance call failed");
    let balance = u128::from_str_radix(result.trim_start_matches("0x"), 16)
        .expect("Failed to decode balance response");
    eprintln!("Current wrapped token balance: {balance}");

    let amount = args.amount.into();
    let input = wrapped_token_api::ERC_20_APPROVE
        .encode_input(&[Token::Address(bft_bridge), Token::Uint(amount)])
        .unwrap();

    let nonce = client
        .account_basic(wallet.address().into())
        .await
        .expect("Failed to get account info.")
        .nonce;
    let approve_tx = TransactionBuilder {
        from: &wallet.address().into(),
        to: Some(token.into()),
        nonce,
        value: 0u64.into(),
        gas: 5_000_000u64.into(),
        gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
        input,
        signature: SigningMethod::SigningKey(wallet.signer()),
        chain_id,
    }
    .calculate_hash_and_build()
    .expect("failed to sign the transaction");

    let hash = client
        .send_raw_transaction(approve_tx)
        .await
        .expect("Failed to send raw transaction")
        .expect("Failed to execute approve transaction");
    wait_for_tx_success(&client, hash).await;

    let input = bft_bridge_api::BURN
        .encode_input(&[
            Token::Uint(amount),
            Token::Address(token),
            Token::FixedBytes(args.to_token_id.into_bytes()),
            Token::Bytes(args.address.into_bytes()),
        ])
        .unwrap();

    let nonce = client
        .account_basic(wallet.address().into())
        .await
        .expect("Failed to get account info.")
        .nonce;
    let burn_tx = TransactionBuilder {
        from: &wallet.address().into(),
        to: Some(bft_bridge.into()),
        nonce,
        value: 0u64.into(),
        gas: 5_000_000u64.into(),
        gas_price: Some((EIP1559_INITIAL_BASE_FEE * 2).into()),
        input,
        signature: SigningMethod::SigningKey(wallet.signer()),
        chain_id,
    }
    .calculate_hash_and_build()
    .expect("failed to sign the transaction");

    let hash = client
        .send_raw_transaction(burn_tx)
        .await
        .expect("Failed to send raw transaction")
        .expect("Failed to execute burn transaction");
    wait_for_tx_success(&client, hash).await;
}

fn decode_token_id(id_string: &str) -> Option<Id256> {
    if let Ok(hex) = hex::decode(id_string) {
        if hex.len() == 32 {
            return Id256::from_slice(&hex);
        }
    }

    let split: Vec<_> = id_string.split(':').collect();
    if split.len() == 2 {
        let block_id = split[0]
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("invalid rune id: {id_string})"));
        let tx_index = split[1]
            .parse::<u32>()
            .unwrap_or_else(|_| panic!("invalid rune id: {id_string})"));
        return Some(Id256::from_btc_tx_index(block_id, tx_index));
    }

    if let Ok(id) = Principal::from_str(id_string) {
        return Some(Id256::from(&id));
    }

    None
}
