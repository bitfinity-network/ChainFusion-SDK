#!/bin/bash

source "$(dirname "$0")/deploy_functions.sh"

IC_NETWORK="local"
BITCOIN_NETWORK="regtest"

function usage() {
  echo "Usage: $0 [options]"
  echo "Options:"
  echo "  -h, --help                                      Display this help message"
  echo "  -b, --bitcoin-network <network>                 Bitcoin network (regtest, testnet, mainnet) (default: regtest)"
  echo "  -e, --evm-principal <principal>                 EVM Principal"
  echo "  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)"
  echo "  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)"
  echo "  --ord-url <ord-url>                             ORD URL"
}

ARGS=$(getopt -o e:b:i:m:h --long evm-principal,ic-network,install-mode,bitcoin-network,ord-url,help -- "$@")
while true; do
  case "$1" in
    -e|--evm-principal)
      EVM_PRINCIPAL="$2"
      shift 2
      ;;

    -i|--ic-network)
      IC_NETWORK="$2"
      shift 2
      ;;

    -b|--bitcoin-network)
      BITCOIN_NETWORK="$2"
      shift 2
      ;;

    -m|--install-mode)
      INSTALL_MODE="$2"
      shift 2
      ;;

    --ord-url)
      ORD_URL="$2"
      shift 2
      ;;

    -h|--help)
      usage
      exit 255
      ;;

    --)
      shift
      break
      ;;

    *)
      break
  esac
done

if [ "$IC_NETWORK" == "local" ]; then
  ORD_URL=${ORD_URL:-"https://127.0.0.1:8001"}
else
  assert_isset_param "$EVM_PRINCIPAL" "EVM_PRINCIPAL"
fi
assert_isset_param "$INSTALL_MODE" "INSTALL_MODE"
assert_isset_param "$BITCOIN_NETWORK" "BITCOIN_NETWORK"
assert_isset_param "$ORD_URL" "ORD_URL"

LOG_SETTINGS="record { enable_console=false; in_memory_records=opt 10000; log_filter=opt \"info,evm_core=debug,evm=info\"; }"
OWNER=$(dfx identity get-principal)
SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Production }; } }"

if [ "$IC_NETWORK" = "local" ]; then
  start_dfx 1
  SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Dfx }; } }"
  EVM_PRINCIPAL=$(deploy_evm_testnet)
fi

if [ "$INSTALL_MODE" = "create" ] || [ "$INSTALL_MODE" = "init" ]; then
  INSTALL_MODE="install"
fi 

if [ "$INSTALL_MODE" != "install" ] && [ "$INSTALL_MODE" != "upgrade" ] && [ "$INSTALL_MODE" != "reinstall" ]; then
  echo "Usage: $0 <create|init|upgrade|reinstall>"
  exit 1
fi

set -e
deploy_btc_nft_bridge "$IC_NETWORK" "$INSTALL_MODE" "$BITCOIN_NETWORK" "$OWNER" "$EVM_PRINCIPAL" "$ORD_URL" "$SIGNING_STRATEGY" "$LOG_SETTINGS"
set +e

if [ "$IC_NETWORK" == "local" ]; then
  start_icx "$EVM_PRINCIPAL"
fi
