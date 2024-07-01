use std::time::Duration;

use did::{H160, U64};
use eth_signer::{Signer, Wallet};
use ethers_core::abi::{Constructor, Param, ParamType, Token};
use ethers_core::k256::ecdsa::SigningKey;
use evm_canister_client::EvmCanisterClient;
use ic_canister_client::CanisterClient as _;
use minter_contract_utils::bft_bridge_api;
use minter_contract_utils::build_data::test_contracts::TEST_WTM_HEX_CODE;
use minter_contract_utils::build_data::{
    BFT_BRIDGE_SMART_CONTRACT_CODE, UUPS_PROXY_SMART_CONTRACT_CODE,
};
use minter_contract_utils::evm_bridge::BridgeSide;
use minter_did::id256::Id256;

use super::DfxTestContext;
use crate::context::{CanisterType, TestContext};
use crate::dfx_tests::ADMIN;
use crate::utils::CHAIN_ID;

#[allow(dead_code)]
pub struct ContextWithBridges {
    pub context: DfxTestContext,
    pub bob_wallet: Wallet<'static, SigningKey>,
    pub bob_address: H160,
    pub erc20_minter_address: H160,
    pub base_bft_bridge: H160,
    pub wrapped_bft_bridge: H160,
    pub base_token_address: H160,
    pub wrapped_token_address: H160,
    pub fee_charge_address: H160,
}

impl ContextWithBridges {
    pub async fn new() -> Self {
        let ctx = DfxTestContext::new(&CanisterType::EVM_MINTER_WITH_EVMRPC_TEST_SET).await;

        // Deploy external EVM canister.
        let base_evm = ctx.canisters().external_evm();
        println!("BASE EVM: {}", base_evm);
        let base_evm_client = EvmCanisterClient::new(ctx.client(base_evm, ctx.admin_name()));

        println!("Deployed external EVM canister: {}", base_evm);
        println!("Deployed EVM canister: {}", ctx.canisters().evm());

        let fee_charge_deployer = ctx.new_wallet(u128::MAX).await.unwrap();
        let deployer_address = fee_charge_deployer.address();
        base_evm_client
            .mint_native_tokens(deployer_address.into(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();
        ctx.advance_time(Duration::from_secs(10)).await;
        let expected_fee_charge_address =
            ethers_core::utils::get_contract_address(fee_charge_deployer.address(), 0);

        let mut rng = rand::thread_rng();

        let bob_wallet = Wallet::new(&mut rng);
        let bob_address: H160 = bob_wallet.address().into();

        // Mint native tokens for bob in both evms
        base_evm_client
            .mint_native_tokens(bob_address.clone(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();
        ctx.evm_client(ADMIN)
            .mint_native_tokens(bob_address.clone(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();
        ctx.advance_time(Duration::from_secs(2)).await;

        // get evm minter canister address
        let erc20_minter_client = ctx.client(ctx.canisters().ck_erc20_minter(), ADMIN);
        let erc20_minter_address = erc20_minter_client
            .update::<_, Option<H160>>("get_evm_address", ())
            .await
            .unwrap()
            .unwrap();

        // mint native tokens for the erc20-minter on both EVMs
        println!("Minting native tokens on both EVMs for {erc20_minter_address}");
        ctx.evm_client(ADMIN)
            .mint_native_tokens(erc20_minter_address.clone(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();
        base_evm_client
            .mint_native_tokens(erc20_minter_address.clone(), u64::MAX.into())
            .await
            .unwrap()
            .unwrap();
        ctx.advance_time(Duration::from_secs(2)).await;

        // Deploy the BFTBridge contract on the external EVM.
        let base_bft_bridge = create_bft_bridge(
            &ctx,
            &bob_wallet,
            BridgeSide::Base,
            expected_fee_charge_address.into(),
            erc20_minter_address.clone(),
        )
        .await;
        println!("Base BFT Bridge: {}", base_bft_bridge);
        let wrapped_bft_bridge = create_bft_bridge(
            &ctx,
            &bob_wallet,
            BridgeSide::Wrapped,
            expected_fee_charge_address.into(),
            erc20_minter_address.clone(),
        )
        .await;
        println!("Wrapped BFT Bridge: {}", wrapped_bft_bridge);

        // Deploy FeeCharge contracts.
        let fee_charge_address = ctx
            .initialize_fee_charge_contract_on_evm(
                &base_evm_client,
                &fee_charge_deployer,
                &[base_bft_bridge.clone()],
            )
            .await
            .unwrap();
        assert_eq!(expected_fee_charge_address, fee_charge_address.0);
        let fee_charge_address = ctx
            .initialize_fee_charge_contract(&fee_charge_deployer, &[wrapped_bft_bridge.clone()])
            .await
            .unwrap();
        assert_eq!(expected_fee_charge_address, fee_charge_address.0);

        // Deploy ERC-20 token on external EVM.
        let data: Constructor = Constructor {
            inputs: vec![Param {
                name: "initialSupply".into(),
                kind: ParamType::Uint(256),
                internal_type: None,
            }],
        };

        let data = data
            .encode_input(TEST_WTM_HEX_CODE.clone(), &[Token::Uint(u64::MAX.into())])
            .unwrap();

        let nonce = base_evm_client
            .account_basic(bob_address.clone())
            .await
            .unwrap()
            .nonce;
        let tx = ctx.signed_transaction(&bob_wallet, None, nonce, 0, data);
        let base_token_address = {
            let hash = base_evm_client
                .send_raw_transaction(tx)
                .await
                .unwrap()
                .unwrap();

            let receipt = ctx
                .wait_transaction_receipt_on_evm(&base_evm_client, &hash)
                .await
                .unwrap()
                .unwrap();

            assert_eq!(receipt.status, Some(U64::one()));

            receipt.contract_address.unwrap()
        };

        // Deploy Wrapped token on first EVM for the ERC-20 from previous step.
        let token_id = Id256::from_evm_address(&base_token_address, CHAIN_ID as _);
        let wrapped_token_address = ctx
            .create_wrapped_token(
                &ctx.new_wallet(u128::MAX).await.unwrap(),
                &wrapped_bft_bridge,
                token_id,
            )
            .await
            .unwrap();

        Self {
            context: ctx,
            bob_wallet,
            bob_address,
            erc20_minter_address,
            base_bft_bridge,
            wrapped_bft_bridge,
            base_token_address,
            wrapped_token_address,
            fee_charge_address,
        }
    }
}

// Create a second EVM canister (base_evm) instance and create BFTBridge contract on it, // It will play role of external evm
// Create erc20-minter instance, initialized with EvmInfos for both EVM canisters.
// Deploy ERC-20 token on external_evm,
// Deploy Wrapped token on first EVM for the ERC-20 from previous step,
// Approve ERC-20 transfer on behalf of some user in external_evm,
// Call BFTBridge::burn() on behalf of the user in external_evm.
// Wait some time for the erc20-minter see and process it.
// Make sure the tokens minted.
// Make sure SignedMintOrder removed from erc20-minter after some time.
#[tokio::test]
async fn test_should_setup_with_evm_rpc_canister() {
    let _ = ContextWithBridges::new().await;
}

async fn create_bft_bridge(
    ctx: &DfxTestContext,
    wallet: &Wallet<'static, SigningKey>,
    side: BridgeSide,
    fee_charge: H160,
    minter_address: H160,
) -> H160 {
    let minter_client = ctx.client(ctx.canisters().ck_erc20_minter(), ADMIN);

    let is_wrapped = match side {
        BridgeSide::Base => false,
        BridgeSide::Wrapped => true,
    };

    let contract = BFT_BRIDGE_SMART_CONTRACT_CODE.clone();
    let input = bft_bridge_api::CONSTRUCTOR
        .encode_input(contract, &[])
        .unwrap();

    let evm = match side {
        BridgeSide::Base => ctx.canisters().external_evm(),
        BridgeSide::Wrapped => ctx.canisters().evm(),
    };

    let evm_client = EvmCanisterClient::new(ctx.client(evm, ADMIN));

    let bridge_address = ctx
        .create_contract_on_evm(&evm_client, wallet, input.clone())
        .await
        .unwrap();

    let initialize_data = bft_bridge_api::proxy::INITIALISER
        .encode_input(&[
            Token::Address(minter_address.0),
            Token::Address(fee_charge.0),
            Token::Bool(is_wrapped),
        ])
        .expect("encode input");

    let proxy_input = bft_bridge_api::proxy::CONSTRUCTOR
        .encode_input(
            UUPS_PROXY_SMART_CONTRACT_CODE.clone(),
            &[
                Token::Address(bridge_address.0),
                Token::Bytes(initialize_data),
            ],
        )
        .unwrap();

    let proxy_address = ctx
        .create_contract_on_evm(&evm_client, wallet, proxy_input)
        .await
        .unwrap();

    minter_client
        .update::<_, ()>("set_bft_bridge_contract", (proxy_address.clone(), side))
        .await
        .unwrap();

    proxy_address
}
