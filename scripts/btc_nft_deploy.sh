#!/bin/bash

# Script to deploy and test the BtcNftBridge canister to perform a complete bridging flow (BTCNFT <> ERC721).
#
# NOTES: 
#
# 1. Version 0.18 of dfx has a bug not allowing BTC operations to work properly. Future versions may fix the issue.
# Until then, this script uses dfx version 0.17.
#
# 2. We use https://127.0.0.1:8001 as the indexer to fetch a reveal transaction by its ID, and to fetch NFT inscription details.
# Notice it's using HTTPS, which is required by `dfx`. Therefore we need to set up two things:
#
#     a. `mkcert`, which automatically creates and installs a local CA in the system root store, and generates locally-trusted certificates.
#         After installing `mkcert` and generating the cert and key, you'll see an output like this:
#             The certificate is at "./localhost+2.pem" and the key at "./localhost+2-key.pem"
#
#     b. an SSL proxy (e.g. `local-ssl-proxy`, `caddy`, etc.) which utilises the cert and key generated previously:
#           local-ssl-proxy --source 8001 --target 8000 -c localhost+2.pem -k localhost+2-key.pem &
#

set +e

# ######################## Start the Bitcoin Daemon and Ord #####################

# echo "Starting the Bitcoin daemon"
# COMPOSE_FILE="../ckERC20/ord-testnet/bridging-flow/docker-compose.yml"

# docker-compose -f "$COMPOSE_FILE" up -d bitcoind
# sleep 2

# wallet_exists=$(docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest listwallets | grep -c "testwallet")

# if [ "$wallet_exists" -eq 0 ]; then
#     echo "Creating 'testwallet'..."
#     docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest createwallet "testwallet"
# fi

# echo "Loading 'testwallet'..."
# load_output=$(docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest loadwallet "testwallet" 2>&1)

# if echo "$load_output" | grep -q "Wallet file verification failed"; then
#     echo "Wallet already exists but could not be loaded due to verification failure."
# elif echo "$load_output" | grep -q "Wallet loaded successfully"; then
#     echo "Wallet loaded successfully."
# else
#     echo "Unexpected wallet load output: $load_output"
# fi

# # Generate 101 blocks to ensure enough coins are available for spending
# height=$(docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest getblockcount)
# if [ "$height" -lt 101 ]; then
#     echo "Generating 101 blocks..."
#     new_address=$(docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest getnewaddress)
#     docker-compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest generatetoaddress 101 "$new_address"
# fi

# # Start the ord service
# echo "Starting the 'ord' service..."
# docker-compose -f "$COMPOSE_FILE" up -d ord
# if [ $? -ne 0 ]; then
#     echo "Failed to start 'ord' service. Attempting to continue script..."
# fi

############################### Configure Dfx #################################

echo "Starting dfx in a clean state"
dfx stop
rm -f dfx_log.txt
dfx start --clean --background --enable-bitcoin  --host 127.0.0.1:4943 >dfx_log.txt 2>&1

dfx identity new --force btc-admin
dfx identity use btc-admin

ADMIN_PRINCIPAL=$(dfx identity get-principal)
ADMIN_WALLET=$(dfx identity get-wallet)

######################### Deploy EVM and NFT Bridge ######################

CHAIN_ID=355113

INDEXER_URL="https://127.0.0.1:8001"
# INDEXER_URL="https://host.docker.internal:8001"

echo "Deploying EVMc testnet"
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
      log_filter = opt \"info,brc20_bridge::scheduler=warn\";
    };
    owner = principal \"${ADMIN_PRINCIPAL}\";
    genesis_accounts = vec { };
    chain_id = $CHAIN_ID;
    coinbase = \"0x0000000000000000000000000000000000000000\";
})"

echo "Deploying BTC NFt bridge"
dfx deploy btc-nft-bridge --argument "(record {
    indexer = \"${INDEXER_URL}\";
    admin = principal \"${ADMIN_PRINCIPAL}\";
    signing_strategy = variant { ManagementCanister = record { key_id = variant { Dfx } } };
    evm_link = variant { Ic = principal \"${EVM}\" };
    network = variant { regtest };
    logger = record {
      enable_console = true;
      in_memory_records = opt 10000;
      log_filter = opt \"info\";
    };
})"

######################## Deploy BFT and Token Contracts ######################

ETH_WALLET=$(cargo run -q -p create_bft_bridge_tool -- create-wallet --evm-canister="$EVM")
ETH_WALLET_ADDRESS=$(cargo run -q -p create_bft_bridge_tool -- wallet-address --wallet="$ETH_WALLET")
ETH_WALLET_CANDID=$(cargo run -q -p create_bft_bridge_tool -- wallet-address --wallet="$ETH_WALLET" --candid)

NFT_BRIDGE=$(dfx canister id btc-nft-bridge)

res=$(dfx canister call btc-nft-bridge get_evm_address)
res=${res#*\"}
NFT_BRIDGE_ETH_ADDRESS=${res%\"*}

echo "NFT bridge eth address: ${NFT_BRIDGE_ETH_ADDRESS}"

echo "Minting ETH tokens for NFT bridge canister"
dfx canister call evm_testnet mint_native_tokens "(\"${NFT_BRIDGE_ETH_ADDRESS}\", \"340282366920938463463374607431768211455\")"

ERC721_ETH_ADDRESS=$(cargo run -q -p create_bft_bridge_tool -- deploy-erc721-bridge --minter-address="$NFT_BRIDGE_ETH_ADDRESS" --evm="$EVM" --wallet="$ETH_WALLET")
echo "NFT ETH address: $ERC721_ETH_ADDRESS"

TOKEN_ETH_ADDRESS=$(cargo run -q -p create_bft_bridge_tool -- create-nft \
  --erc721-bridge-address="$ERC721_ETH_ADDRESS" \
  --token-name=KOBP \
  --token-id="$NFT_BRIDGE" \
  --evm-canister="$EVM" \
  --wallet="$ETH_WALLET")

echo "Wrapped token ETH address: $TOKEN_ETH_ADDRESS"

echo "Configuring NFT bridge canister"
dfx canister call btc-nft-bridge admin_configure_nft_bridge "(record {
  token_symbol = vec { 42; 54; 43; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; };
  token_address = \"$TOKEN_ETH_ADDRESS\";
  bridge_address = \"$ERC721_ETH_ADDRESS\";
  erc721_chain_id = 355113;
  token_name = vec { 42; 54; 43; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; 0; };
})"

echo "All canisters successfully deployed."

######################## Swap NFT for ERC721 ######################

echo "Preparing to bridge a NFT inscription to an ERC721 token"

# 1. Get canister's deposit address
nft_bridge_addr=$(dfx canister call btc-nft-bridge get_deposit_address)
echo "NFT bridge BTC address (raw output): $nft_bridge_addr"

BRIDGE_ADDRESS=$(echo "$nft_bridge_addr" | sed -e 's/.*"\(.*\)".*/\1/')
echo "NFT bridge canister BTC address: $BRIDGE_ADDRESS"

# 2. Top up the canister's balance to enable it send transactions
CONTAINER_ID=$(docker ps -q --filter "name=bitcoind")

if [ -z "$CONTAINER_ID" ]; then
    echo "bitcoind container not found. Make sure it is running."
    exit 1
fi

echo "bitcoind container ID: $CONTAINER_ID"

# echo "Topping up canister's wallet"
# docker exec "$CONTAINER_ID" bitcoin-cli -regtest generatetoaddress 1 "$BRIDGE_ADDRESS"

# # 3. Check balance
# dfx canister call btc-nft-bridge get_balance "(\"$BRIDGE_ADDRESS\")"

# # 4. Prepare and make the NFT inscription
# NFT_TICKER="kobp"
# brc20_inscription="{\"p\":\"brc-20\", \"op\":\"deploy\", \"tick\":\"$NFT_TICKER\", \"max\":\"1000\", \"lim\":\"10\", \"dec\":\"8\"}"

# echo "Creating a NFT inscription with content: $brc20_inscription"
# dfx canister call btc-nft-bridge inscribe "(variant { Brc20 }, \"${brc20_inscription//\"/\\\"}\", \"${BRIDGE_ADDRESS}\", \"${BRIDGE_ADDRESS}\", null)"

# # 5. Swap the NFT inscription for an ERC20 token
# for i in 1 2 3; do
#   sleep 5
#   echo "Trying to bridge from NFT to ERC20"
#   mint_status=$(dfx canister call btc-nft-bridge brc20_to_erc20 "(\"$NFT_TICKER\", \"$BRIDGE_ADDRESS\", \"$ETH_WALLET_ADDRESS\")")
#   echo "Result: $mint_status"

#   if [[ $mint_status == *"Minted"* ]]; then
#     echo "Minting of ERC20 token successful."
#     break
#   fi

#   if [[ $i -eq 3 ]]; then
#     echo "Failed to mint after 3 retries"
#     exit 1
#   fi
# done

# sleep 5

# ######################## Swap ERC20 for NFT ######################
# USER_ADDRESS=$(docker exec -it "$CONTAINER_ID" bitcoin-cli -regtest -rpcwallet="testwallet" listreceivedbyaddress 0 true | jq -r '.[0].address')
# echo "Inscription destination and leftovers address: $USER_ADDRESS"

# cargo run -q -p create_bft_bridge_tool -- burn-wrapped \
#   --wallet="$ETH_WALLET" \
#   --evm-canister="$EVM" \
#   --bft-bridge="$ERC721_ETH_ADDRESS" \
#   --token-address="$TOKEN_ETH_ADDRESS" \
#   --address="$USER_ADDRESS" \
#   --amount=10