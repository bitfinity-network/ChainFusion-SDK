#!/bin/bash

source "$(dirname "$0")/deploy_functions.sh"

IC_NETWORK="local"

function usage() {
  echo "Usage: $0 [options]"
  echo "Options:"
  echo "  -h, --help                                      Display this help message"
  echo "  -e, --evm-rpc-url <principal>                   EVM RPC URL canister Principal"
  echo "  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)"
  echo "  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)"
  echo "  --base-evm <canister-id>                        Base EVM link canister ID"
  echo "  --wrapped-evm <canister-id>                     Wrapped EVM link canister ID"
  echo "  --erc20-base-bridge-contract <address>          ERC20 Base bridge contract address"
  echo "  --erc20-wrapped-bridge-contract <address>       ERC20 Wrapped bridge contract address"
}

ARGS=$(getopt -o e:i:m:h --long evm-rpc-url,ic-network,install-mode,base-evm,wrapped-evm,erc20-base-bridge-contract,erc20-wrapped-bridge-contract,help -- "$@")
while true; do
  case "$1" in
    -e|--evm-rpc-url)
      EVM_RPC_PRINCIPAL="$2"
      shift 2
      ;;

  -i | --ic-network)
    IC_NETWORK="$2"
    shift 2
    ;;

  -m | --install-mode)
    INSTALL_MODE="$2"
    shift 2
    ;;

  --base-evm)
    BASE_EVM_LINK=$(link_to_variant "$2")
    shift 2
    ;;

  --wrapped-evm)
    WRAPPED_EVM_LINK=$(link_to_variant "$2")
    shift 2
    ;;

  --erc20-base-bridge-contract)
    BASE_BRIDGE_CONTRACT="$2"
    shift 2
    ;;

  --erc20-wrapped-bridge-contract)
    WRAPPED_BRIDGE_CONTRACT="$2"
    shift 2
    ;;

  -h | --help)
    usage
    exit 255
    ;;

  --)
    shift
    break
    ;;

  *)
    break
    ;;
  esac
done

assert_isset_param "$INSTALL_MODE" "INSTALL_MODE"
if [ "$IC_NETWORK" != "local" ]; then
  assert_isset_param "$EVM_RPC_PRINCIPAL" "EVM_RPC_PRINCIPAL"
  assert_isset_param "$BASE_EVM_LINK" "BASE_EVM_LINK"
  assert_isset_param "$WRAPPED_EVM_LINK" "WRAPPED_EVM_LINK"
  assert_isset_param "$WRAPPED_BRIDGE_CONTRACT" "WRAPPED_BRIDGE_CONTRACT"
  assert_isset_param "$BASE_BRIDGE_CONTRACT" "BASE_BRIDGE_CONTRACT"
fi

# get positional arguments; skip $0, if empty 'all'
CANISTERS_TO_DEPLOY="${@:1}"
if [ -z "$CANISTERS_TO_DEPLOY" ]; then
  CANISTERS_TO_DEPLOY="icrc2-minter erc20-minter rune-bridge btc-bridge"
fi

LOG_SETTINGS="opt record { enable_console=false; in_memory_records=opt 10000; log_filter=opt \"info,evm_core=debug,evm=info\"; }"
OWNER=$(dfx identity get-principal)
SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Production }; } }"

if [ "$IC_NETWORK" = "local" ]; then
  SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Dfx }; } }"
  start_dfx
  # deploy evm-rpc canister
  dfx deps pull
  dfx deps init evm_rpc --argument '(record { nodesInSubnet = 28 })'
  dfx deps deploy

  WRAPPED_EVM_PRINCIPAL=$(deploy_evm_testnet)
  echo "Wrapped EVM Principal: $WRAPPED_EVM_PRINCIPAL"
  if [ -z "$ICRC2_MINTER_ID" ]; then
    deploy_icrc2_minter "local" "install" "$WRAPPED_EVM_PRINCIPAL" "$OWNER" "$SIGNING_STRATEGY" "$LOG_SETTINGS"
  fi
  MINTER_ADDRESS=$(dfx canister call icrc2-minter get_minter_canister_evm_address)
  echo "Minter Address: $MINTER_ADDRESS"
  MINTER_ADDRESS=${MINTER_ADDRESS#*\"}
  MINTER_ADDRESS=${MINTER_ADDRESS%\"*}

  # get base bridge contract
  EVM_RPC_PRINCIPAL="$(dfx canister id evm_rpc)"
  WALLET=$(get_wallet $WRAPPED_EVM_PRINCIPAL)
  echo "Deploying bridge $WRAPPED_EVM_PRINCIPAL"
  BASE_BRIDGE_CONTRACT=$(deploy_bft_bridge $WRAPPED_EVM_PRINCIPAL $MINTER_ADDRESS $WALLET)
  if [ -z "$BASE_BRIDGE_CONTRACT" ]; then
    echo "Failed to deploy base bridge contract"
    exit 1
  fi
  # get wrapped bridge contract
  WRAPPED_BRIDGE_CONTRACT=$(deploy_bft_bridge $WRAPPED_EVM_PRINCIPAL $MINTER_ADDRESS $WALLET)
  if [ -z "$WRAPPED_BRIDGE_CONTRACT" ]; then
    echo "Failed to deploy wrapped bridge contract"
    exit 1
  fi
  SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Dfx }; } }"
  ICRC2_MINTER_ID=$(dfx canister id icrc2-minter)
  LOCAL_EVM_LINK="variant { EvmRpcCanister = record { canister_id = principal \"$EVM_RPC_PRINCIPAL\"; rpc_service = vec { variant { Custom = record { url = \"http://127.0.0.1:8545\"; headers = opt null } } } } }"
else
  LOCAL_EVM_LINK="variant { EvmRpcCanister = record { canister_id = principal \"$EVM_RPC_PRINCIPAL\"; rpc_service = vec { variant { EthMainnet = variant { Cloudflare } } } } }"
  # Get FEE_CHARGE_ADDRESS
  FEE_CHARGE_DEPLOY_TX_NONCE=0
  FEE_CHARGE_CONTRACT_ADDRESS=$(cargo run -q -p bridge-tool -- expected-contract-address --wallet="$ETH_WALLET" --nonce=$FEE_CHARGE_DEPLOY_TX_NONCE)

  # get base bridge contract
  WALLET=$(get_wallet $EVM_PRINCIPAL)
  IS_WRAPPED="false"
  BASE_BRIDGE_CONTRACT=$(deploy_bft_bridge $EVM_PRINCIPAL $WALLET $MINTER_ADDRESS $FEE_CHARGE_CONTRACT_ADDRESS "$IS_WRAPPED")
  # get wrapped bridge contract
  IS_WRAPPED="true"
  WRAPPED_EVM_PRINCIPAL=$EVM_PRINCIPAL
  WRAPPED_BRIDGE_CONTRACT=$(deploy_bft_bridge $WRAPPED_EVM_PRINCIPAL $WALLET $MINTER_ADDRESS $FEE_CHARGE_CONTRACT_ADDRESS "$IS_WRAPPED")
  echo "Wrapped EVM Principal: $WRAPPED_EVM_PRINCIPAL"

  echo "Deploying FeeCharge contract"

  BRIDGES=($BASE_BRIDGE_CONTRACT $WRAPPED_BRIDGE_CONTRACT)

  deploy_fee_charge_contract $EVM $ETH_WALLET $FEE_CHARGE_DEPLOY_TX_NONCE $BRIDGES

fi



if [ "$INSTALL_MODE" = "create" ] || [ "$INSTALL_MODE" = "init" ]; then
  INSTALL_MODE="install"
fi

if [ "$INSTALL_MODE" != "install" ] && [ "$INSTALL_MODE" != "upgrade" ] && [ "$INSTALL_MODE" != "reinstall" ]; then
  echo "Usage: $0 <create|init|upgrade|reinstall>"
  exit 1
fi

set -e
deploy_erc20_minter "$IC_NETWORK" "$INSTALL_MODE" "$LOCAL_EVM_LINK" "$WRAPPED_EVM_PRINCIPAL" "$BASE_BRIDGE_CONTRACT" "$WRAPPED_BRIDGE_CONTRACT" "$SIGNING_STRATEGY" "$LOG_SETTINGS"
set +e

if [ "$IC_NETWORK" == "local" ]; then
  start_icx "$WRAPPED_EVM_PRINCIPAL"
fi
