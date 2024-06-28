# Use latets vers
source "$(dirname "$0")/deploy/deploy_functions.sh"

set -e

start_icx() {
    killall icx-proxy
    sleep 2
    # Start ICX Proxy
    dfx_local_port=$(dfx info replica-port)
    icx-proxy --fetch-root-key --address 127.0.0.1:8545 --dns-alias 127.0.0.1:$EVM --replica http://localhost:$dfx_local_port &
    sleep 2

    curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc": "2.0", "method": "eth_chainId", "params": [], "id":1}' 'http://127.0.0.1:8545'
}

CHAIN_ID=355113

dfx stop
dfx start --host 127.0.0.1:4943 --background --clean 2>dfx_stderr.log

dfx identity new --force icrc-admin
dfx identity use icrc-admin

ADMIN_PRINCIPAL=$(dfx identity get-principal)
ADMIN_WALLET=$(dfx identity get-wallet)

########## Deploy  canisters ##########

dfx canister create token2

ICRC2_TOKEN=$(dfx canister id token2)

USER_PRINCIPICAL="qhjy5-udjmu-rqh6d-abbcl-63l4x-gbwoi-ip5qu-vocyn-docsu-fideu-eae"
dfx deploy token2 --argument "(variant {Init = record {
    minting_account = record { owner = principal \"$ADMIN_PRINCIPAL\" };
    transfer_fee = 10;
    token_symbol = \"AUX\";
    token_name = \"Aux Token\";
    metadata = vec {};
    initial_balances = vec {
        record {
            record {
                owner = principal \"$USER_PRINCIPICAL\";
                subaccount = null;
            };
            100_000_000_000
        }
    };
    max_memo_length = opt 100;
    archive_options = record {
        num_blocks_to_archive = 1000;
        trigger_threshold = 2000;
        max_message_size_bytes = null;
        cycles_for_archive_creation = opt 1_000_000_000_000;
        node_max_memory_size_bytes = opt 3_221_225_472;
        controller_id = principal \"$ADMIN_WALLET\"
    }
}})"

########## Deploy EVM, ICRC2 Minter ##########

dfx canister create evm_testnet
dfx canister create icrc2-minter
EVM=$(dfx canister id evm_testnet)
ICRC2_MINTER=$(dfx canister id icrc2-minter)

dfx deploy signature_verification --argument "(vec { principal \"${EVM}\" })"
SIGNATURE_VERIFICATION=$(dfx canister id signature_verification)

dfx deploy evm_testnet --argument "(record {
    min_gas_price = 10;
    signature_verification_principal = principal \"${SIGNATURE_VERIFICATION}\";
    log_settings = opt record {
        enable_console = true;
        in_memory_records = opt 10000;
        log_filter = opt \"error,did=debug,evm_core=debug,evm=debug\";
    };
    owner = principal \"${ADMIN_PRINCIPAL}\";
    genesis_accounts = vec { };
    chain_id = $CHAIN_ID;
    coinbase = \"0x0000000000000000000000000000000000000000\";
})"

dfx deploy icrc2-minter --argument "(record {
    evm_principal = principal \"$EVM\";
    signing_strategy = variant {
        Local = record {
            private_key = blob \"\\01\\23\\45\\67\\89\\01\\23\\45\\67\\01\\01\\23\\45\\67\\89\\01\\23\\45\\67\\01\\01\\23\\45\\67\\89\\01\\23\\45\\67\\01\\67\\01\";
        }
    };
    log_settings = opt record {
        enable_console = true;
        log_filter = opt \"trace\";
        in_memory_records = opt 10000;
    };
    owner = principal \"$ADMIN_PRINCIPAL\";
})"

start_icx

########## Deploy BFT and ICRC2 contracts ##########
ETH_WALLET=$(cargo run -q -p bridge-tool -- create-wallet --evm-canister="$EVM")
ETH_WALLET_ADDRESS=$(cargo run -q -p bridge-tool -- wallet-address --wallet="$ETH_WALLET")

FEE_CHARGE_DEPLOY_TX_NONCE=2
FEE_CHARGE_CONTRACT_ADDRESS=$(cargo run -q -p bridge-tool -- expected-contract-address --wallet="$ETH_WALLET" --nonce=$FEE_CHARGE_DEPLOY_TX_NONCE)

res=$(dfx canister call icrc2-minter get_minter_canister_evm_address)
res=${res#*\"}
ICRC2_MINTER_ECDSA_ADDRESS=${res%\"*}

echo "ICRC2 Minter ecdsa address: ${ICRC2_MINTER_ECDSA_ADDRESS}"

echo "Minting ETH tokens for ICRC2 Minter canister"
dfx canister call evm_testnet mint_native_tokens "(\"${ICRC2_MINTER_ECDSA_ADDRESS}\", \"340282366920938463463374607431768211455\")"

echo "Deploying BftBridge contract"

IS_WRAPPED="false"
BFT_BRIDGE_ADDRESS=$(deploy_bft_bridge $EVM $ETH_WALLET $ICRC2_MINTER_ECDSA_ADDRESS $FEE_CHARGE_CONTRACT_ADDRESS $IS_WRAPPED)

echo "Got BftBridge address: ${BFT_BRIDGE_ADDRESS}"

echo "Setting ICRC2 Minter BftBridge address"
dfx canister call icrc2-minter set_bft_bridge_contract "(\"${BFT_BRIDGE_ADDRESS}\")"
sleep 5

res=$(dfx canister call icrc2-minter get_bft_bridge_contract)
res=${res#*\"}
BFT_BRIDGE_ADDRESS=${res%\"*}
echo "Got BftBridge address: ${BFT_BRIDGE_ADDRESS}"

echo "Deploying FeeCharge contract"
deploy_fee_charge_contract $EVM $ETH_WALLET $FEE_CHARGE_DEPLOY_TX_NONCE $BFT_BRIDGE_ADDRESS
sleep 5

echo "Finished!!!!!"
