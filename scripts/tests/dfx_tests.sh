#!/bin/bash

export ORD_BITCOIN_RPC_USERNAME=ic-btc-integration
export ORD_BITCOIN_RPC_PASSWORD="QPQiNaph19FqUsCrBRN0FII7lyM26B51fAMeBQzCb-E="
LOGFILE=./target/dfx_tests.log

usage() {
  echo "Usage: $0 [options]"
  echo "Options:"
  echo "  -h, --help                                      Display this help message"
  echo "  --docker                                        Setup docker containers"
  echo "  --github-ci                                     Use this flag when running in GitHub CI"
}

setup_docker() {
    PREV_PATH=$(pwd)
    cd btc-deploy/
    docker compose up -d --build
    cd $PREV_PATH
}

stop_docker() {
    PREV_PATH=$(pwd)
    cd btc-deploy/
    docker compose down
    cd $PREV_PATH
}

WITH_DOCKER="0"
GITHUB_CI="0"

ARGS=$(getopt -o h --long docker,github-ci,help -- "$@")
while true; do
  case "$1" in
  --docker)
    DOCKER="1"
    shift
    ;;

  --github-ci)
    GITHUB_CI="1"
    shift
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

# set dfxvm to use the correct version
if [ "$GITHUB_CI" -gt 0 ]; then
    dfxvm default 0.18.0
fi


killall -9 icx-proxy || true
dfx stop

if [ "$WITH_DOCKER" -gt 0 ]; then
    setup_docker
fi

set -e

start_icx() {
    killall icx-proxy
    sleep 2
    # Start ICX Proxy
    dfx_local_port=$(dfx info replica-port)
    icx-proxy --fetch-root-key --address 0.0.0.0:8545 --dns-alias 0.0.0.0:bd3sg-teaaa-aaaaa-qaaba-cai --replica http://localhost:$dfx_local_port &
    sleep 2
}

rm -f "$LOGFILE"

set +e
dfx start --background --clean --enable-bitcoin 2> "$LOGFILE"
start_icx

dfx identity use max
wallet_principal=$(dfx identity get-wallet)
echo "Wallet Principal: $wallet_principal"
dfx ledger fabricate-cycles --t 1000000 --canister $wallet_principal

sleep 10

cargo test -p integration-tests --features dfx_tests $1

killall -9 icx-proxy || true

dfx stop

if [ "$WITH_DOCKER" -gt 0 ]; then
    stop_docker
fi
