#!/bin/bash

source "$(dirname "$0")/deploy_functions.sh"

IC_NETWORK="local"

function usage() {
  echo "Usage: $0 [options]"
  echo "Options:"
  echo "  -h, --help                                      Display this help message"
  echo "  -e, --evm-principal <principal>                 EVM Principal"
  echo "  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)"
  echo "  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)"
}

ARGS=$(getopt -o e:i:m:h --long evm-principal,ic-network,install-mode,help -- "$@")
while true; do
  case "$1" in
  -e | --evm-principal)
    EVM_PRINCIPAL="$2"
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
  assert_isset_param "$EVM_PRINCIPAL" "EVM_PRINCIPAL"
fi

LOG_SETTINGS="record { enable_console=false; in_memory_records = opt 10000: opt nat64; log_filter=opt \"info, icrc2_minter::tasks=trace\"; }"
OWNER=$(dfx identity get-principal)
SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Production }; } }"

if [ "$IC_NETWORK" = "local" ]; then
  start_dfx
  EVM_PRINCIPAL=$(deploy_evm_testnet)
  SIGNING_STRATEGY="variant { ManagementCanister = record { key_id = variant { Dfx }; } }"
fi

if [ "$INSTALL_MODE" = "create" ] || [ "$INSTALL_MODE" = "init" ]; then
  INSTALL_MODE="install"
elif [ "$INSTALL_MODE" != "install" ] && [ "$INSTALL_MODE" != "upgrade" ] && [ "$INSTALL_MODE" != "reinstall" ]; then
  echo "Usage: $0 <create|init|upgrade|reinstall>"
  exit 1
fi

set -e
deploy_icrc2_minter "$IC_NETWORK" "$INSTALL_MODE" "$EVM_PRINCIPAL" "$OWNER" "$SIGNING_STRATEGY" "$LOG_SETTINGS"
set +e

if [ "$IC_NETWORK" = "local" ]; then
  start_icx "$EVM_PRINCIPAL"
fi
