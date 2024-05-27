#!/bin/bash

CREATE_BFT_BRIDGE_TOOL="cargo run -q -p create_bft_bridge_tool --"

assert_isset_param () {
  PARAM=$1
  NAME=$2
  if [ -z "$PARAM" ]; then
    echo "$NAME is required"
    usage
    exit 1
  fi
}

link_to_variant() {
  LINK=$1
  if [[ $URL == "http"* ]]; then
    IC_TYPE="variant { Http = \"${LINK}\" }"
  else
    IC_TYPE="variant { Ic = principal \"${LINK}\" }"
  fi
  echo "$IC_TYPE"
}

create_canister() {
    # Create canisters
    NETWORK=$1
    CANISTER=$2

    dfx canister --network=$NETWORK create --with-cycles=600000000000 $CANISTER
}

deploy_icrc2_minter() {
  NETWORK="$1"
  INSTALL_MODE="$2"
  EVM_PRINCIPAL="$3"
  ADMIN_PRINCIPAL="$4"
  SIGNING_STRATEGY="$5"
  LOG_SETTINGS="$6"

  create_canister $NETWORK icrc2-minter

  args="(record {
    evm_principal = principal \"$EVM_PRINCIPAL\";
    signing_strategy = $SIGNING_STRATEGY;
    log_settings = opt $LOG_SETTINGS;
    owner = principal \"$ADMIN_PRINCIPAL\";
  })"

  echo "deploying icrc2-minter with args: $args"

  dfx canister install --mode=$INSTALL_MODE --wasm=./.artifact/icrc2-minter.wasm.gz --yes --network=$NETWORK --argument="$args" icrc2-minter
}

deploy_erc20_minter() {
  NETWORK="$1"
  INSTALL_MODE="$2"
  BASE_EVM_LINK=$(link_to_variant "$3")
  WRAPPED_EVM_LINK=$(link_to_variant "$4")
  BASE_BRIDGE_CONTRACT="$5"
  WRAPPED_BRIDGE_CONTRACT="$6"
  SIGNING_STRATEGY="$7"
  LOG_SETTINGS="$8"

  create_canister $NETWORK erc20-minter

  args="(record {
    base_evm_link = $BASE_EVM_LINK;
    wrapped_evm_link = $WRAPPED_EVM_LINK;
    base_bridge_contract = \"$BASE_BRIDGE_CONTRACT\";
    wrapped_bridge_contract = \"$WRAPPED_BRIDGE_CONTRACT\";
    signing_strategy = $SIGNING_STRATEGY;
    log_settings = opt $LOG_SETTINGS;
  })"

  echo "deploying erc20-minter with args: $args"

  dfx canister install --mode=$INSTALL_MODE --yes --wasm=./.artifact/erc20-minter.wasm.gz --network=$NETWORK --argument="$args" erc20-minter
}

deploy_rune_bridge() {
  NETWORK="$1"
  INSTALL_MODE="$2"
  BITCOIN_NETWORK="$3"
  EVM_LINK=$(link_to_variant "$4")
  ADMIN_PRINCIPAL="$5"
  INDEXER_URL="$6"
  SIGNING_STRATEGY="$7"
  LOG_SETTINGS="$8"

  create_canister $NETWORK rune-bridge

  args="(record {
    network = variant { $BITCOIN_NETWORK };
    evm_link = $EVM_LINK;
    signing_strategy = $SIGNING_STRATEGY;
    admin = principal \"$ADMIN_PRINCIPAL\";
    log_settings = opt $LOG_SETTINGS;
    min_confirmations = 1;
    indexer_url = \"$INDEXER_URL\";
  })"

  echo "deploying rune-bridge with args: $args"
  
  dfx canister install --mode=$INSTALL_MODE --yes --wasm=./.artifact/rune-bridge.wasm.gz --network=$NETWORK --argument="$args" rune-bridge
}

deploy_btc_bridge() {
  NETWORK="$1"
  INSTALL_MODE="$2"
  BITCOIN_NETWORK="$3"
  ADMIN_PRINCIPAL="$4"
  EVM_LINK=$(link_to_variant "$5")
  CKBTC_MINTER="$6"
  CKBTC_LEDGER="$7"
  SIGNING_STRATEGY="$8"
  LOG_SETTINGS="$9"
  LEDGER_FEE="${10:-10}"

  create_canister $NETWORK btc-bridge

  args="(record {
    ck_btc_ledger = principal \"$CKBTC_LEDGER\";
    ck_btc_minter = principal \"$CKBTC_MINTER\";
    network = variant { $BITCOIN_NETWORK };
    evm_link = $EVM_LINK;
    signing_strategy = $SIGNING_STRATEGY;
    admin = principal \"$ADMIN_PRINCIPAL\";
    ck_btc_ledger_fee = $LEDGER_FEE;
    log_settings = $LOG_SETTINGS;
  })"

  echo "deploying btc-bridge with args: $args"

  dfx canister install --mode=$INSTALL_MODE --yes --wasm=./.artifact/btc-bridge.wasm.gz --network=$NETWORK --argument="$args" btc-bridge
}

deploy_btc_nft_bridge() {
  NETWORK="$1"
  INSTALL_MODE="$2"
  BITCOIN_NETWORK="$3"
  ADMIN_PRINCIPAL="$4"
  EVM_LINK=$(link_to_variant "$5")
  ORD_URL="$6"
  SIGNING_STRATEGY="$7"
  LOG_SETTINGS="$8"

  create_canister $NETWORK btc-nft-bridge

  args="(record {
    ord_url = \"${ORD_URL}\";
    admin = principal \"${ADMIN_PRINCIPAL}\";
    signing_strategy = $SIGNING_STRATEGY;
    evm_link = $EVM_LINK;
    network = variant { $BITCOIN_NETWORK };
    logger = $LOG_SETTINGS;
  })"

  echo "deploying btc-nft-bridge with args: $args"

  dfx canister install --mode=$INSTALL_MODE --yes --wasm=./.artifact/btc-nft-bridge.wasm.gz --network=$NETWORK --argument="$args" btc-nft-bridge
}

get_wallet() {
  EVM_PRINCIPAL="$1"
  ETH_WALLET=$($CREATE_BFT_BRIDGE_TOOL create-wallet --evm-canister="$EVM_PRINCIPAL")

  echo "$ETH_WALLET"
}

deploy_bft_bridge() {
  EVM_PRINCIPAL="$1"
  MINTER_ADDRESS="$2"
  WALLET="$3"

  BRIDGE_ADDRESS=$($CREATE_BFT_BRIDGE_TOOL deploy-bft-bridge --minter-address="$MINTER_ADDRESS" --evm="$EVM_PRINCIPAL" --wallet="$WALLET")

  echo "$BRIDGE_ADDRESS"
}

deploy_erc721_bridge() {
  EVM_PRINCIPAL="$1"
  MINTER_ADDRESS="$2"
  WALLET="$3"

  BRIDGE_ADDRESS=$($CREATE_BFT_BRIDGE_TOOL deploy-erc721-bridge --minter-address="$MINTER_ADDRESS" --evm="$EVM_PRINCIPAL" --wallet="$WALLET")

  echo "$BRIDGE_ADDRESS"
}

# test canisters


deploy_evm_testnet() {
  set -e
  CHAIN_ID=355113
  ADMIN_PRINCIPAL=$(dfx identity get-principal)
  dfx canister create evm_testnet
  EVM=$(dfx canister id evm_testnet)
  dfx deploy signature_verification --argument "(vec { principal \"${EVM}\" })"
  SIGNATURE_VERIFICATION=$(dfx canister id signature_verification)

  dfx deploy evm_testnet --argument "(record {
      min_gas_price = 10;
      signature_verification_principal = principal \"${SIGNATURE_VERIFICATION}\";
      log_settings = opt record {
          enable_console = true;
          in_memory_records = opt 10000;
          log_filter = opt \"warn\";
      };
      owner = principal \"${ADMIN_PRINCIPAL}\";
      genesis_accounts = vec { };
      chain_id = $CHAIN_ID;
      coinbase = \"0x0000000000000000000000000000000000000000\";
  })"

  set +e

  echo "$EVM"
}

deploy_ckbtc_ledger() {
  set -e
  dfx canister create ic-ckbtc-ledger
  dfx canister create ic-ckbtc-minter
  CKBTC_MINTER=$(dfx canister id ic-ckbtc-minter)
  CKBTC_LEDGER=$(dfx canister id ic-ckbtc-ledger)
  ADMIN_WALLET=$(dfx identity get-wallet)

  dfx deploy token --argument "(variant {Init = record {
    minting_account = record { owner = principal \"$CKBTC_MINTER\" };
    transfer_fee = 10;
    token_symbol = \"ckTESTBTC\";
    token_name = \"Chain key testnet Bitcoin\";
    metadata = vec {};
    initial_balances = vec {};
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

  set +e

  echo "$CKBTC_LEDGER"
}

deploy_ckbtc_kyt() {
  set -e
  CKBTC_MINTER="$(dfx canister id ic-ckbtc-minter)"
  ADMIN_PRINCIPAL=$(dfx identity get-principal)
  dfx canister create ic-ckbtc-kyt
  CKBTC_KYT=$(dfx canister id ic-ckbtc-ledger)
  dfx deploy ic-ckbtc-kyt --argument "(variant {InitArg = record {
    api_key = \"abcdef\";
    maintainers = vec { principal \"$ADMIN_PRINCIPAL\"; };
    mode = variant { AcceptAll };
    minter_id = principal \"$CKBTC_MINTER\";
  } })"

  dfx canister call ic-ckbtc-kyt set_api_key "(record { api_key = \"abc\"; })"

  set +e

  echo "$CKBTC_KYT"
}

deploy_ckbtc_minter() {
  set -e
  CKBTC_LEDGER="$(dfx canister id ic-ckbtc-ledger)"
  CKBTC_MINTER="$(dfx canister id ic-ckbtc-minter)"
  dfx deploy ic-ckbtc-minter --argument "(variant {Init = record {
    btc_network = variant { Regtest };
    ledger_id = principal \"$CKBTC_LEDGER\";
    ecdsa_key_name = \"dfx_test_key\";
    retrieve_btc_min_amount = 5_000;
    max_time_in_queue_nanos = 420_000_000_000;
    min_confirmations = opt 1;
    kyt_fee = opt 1000;
    kyt_principal = opt principal \"$CKBTC_KYT\";
    mode = variant { GeneralAvailability };
  }})"

  set +e

  echo "$CKBTC_MINTER"
}


start_dfx() {
  echo "Attempting to create Alice's Identity"
  set +e

  ENABLE_BITCOIN="${1:-0}"
  if [ "$ENABLE_BITCOIN" -gt 0 ]; then
    ENABLE_BITCOIN="--enable-bitcoin"
  else
    ENABLE_BITCOIN=""
  fi

  if [ "$INSTALL_MODE" = "create" ]; then
    echo "Stopping DFX"
    dfx stop
    echo "Starting DFX"
    dfx start --clean --background $ENABLE_BITCOIN --artificial-delay 0 2> dfx_stderr.log
  else
    return
  fi

  # Create identity
  dfx identity new --force --storage-mode=plaintext alice
  dfx identity use alice
  echo "Alice's Identity Created"
}

start_icx() {
  evm_id="$1"
  killall icx-proxy
  sleep 2
  # Start ICX Proxy
  dfx_local_port=$(dfx info replica-port)
  icx-proxy --fetch-root-key --address 127.0.0.1:8545 --dns-alias 127.0.0.1:$evm_id --replica http://localhost:$dfx_local_port &
  sleep 2

  curl -X POST -H "Content-Type: application/json" --data '{"jsonrpc": "2.0", "method": "eth_chainId", "params": [], "id":1}' 'http://127.0.0.1:8545'
}

