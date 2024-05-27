# Deploy

- [Deploy](#deploy)
  - [Requirements](#requirements)
  - [Build](#build)
  - [Canisters deployment](#canisters-deployment)
    - [BFT Bridge](#bft-bridge)
    - [BTC Bridge](#btc-bridge)
    - [BTC NFT Bridge](#btc-nft-bridge)
    - [ERC20 Bridge](#erc20-bridge)
    - [ERC721 Bridge](#erc721-bridge)
    - [ICRC2 Minter](#icrc2-minter)
    - [Rune Bridge](#rune-bridge)

## Requirements

- [Get `dfx` here](https://internetcomputer.org/docs/current/developer-docs/getting-started/install/#installing-dfx) if you don't have it already.
- [Install the Rust toolchain](https://www.rust-lang.org/tools/install) if it's not already installed.
- [Download and install Docker, with Compose](https://www.docker.com/products/docker-desktop/) if you don't already have it.
- Install [foundry](https://book.getfoundry.sh/getting-started/installation).

    ```sh
    curl -L https://foundry.paradigm.xyz | bash
    ```

## Build

Before deploying the canister, you need to build both the canisters and the solidity contracts first.

```sh
./scripts/build_solidity.sh
./scripts/build.sh
```

## Canisters deployment

> ❗ All the scripts, if run against localhost and with `-m create`, will deploy the evm first

### BFT Bridge

```sh
./scripts/deploy/bft-bridge.sh
```

```txt
Options:
  -h, --help                                      Display this help message
  -e, --evm-canister <principal>                  EVM canister principal
  -w, --wallet <ETH wallet address>               Ethereum wallet address for deploy
  --minter-address <minter-address>               Bridge minter address
  --dfx-setup                                     Setup dfx locally
```

In order to deploy the BFT bridge, run:

If deploying on ic

```sh
./scripts/deploy/bft-bridge.sh -e <evm-principal> -m <minter-address> -w <wallet-address>
```

If deploying on localhost just pass the `--dfx-setup` option.

```sh
./scripts/deploy/bft-bridge.sh --dfx-setup
```

### BTC Bridge

Deploy the Bitcoin bridge

```sh
./scripts/deploy/btc-bridge.sh --help
```

```txt
Options:
  -h, --help                                      Display this help message
  -b, --bitcoin-network <network>                 Bitcoin network (regtest, testnet, mainnet) (default: regtest)
  -e, --evm-principal <principal>                 EVM Principal
  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)
  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)
  --ckbtc-minter <canister-id>                    CK-BTC minter canister ID
  --ckbtc-ledger <canister-id>                    CK-BTC ledger canister ID
```

If deploying on ic

```sh
./scripts/deploy/btc-bridge.sh -m <create|install|reinstall|update> -i ic -e <evm-principal> --bitcoin-network mainnet
```

If deploying on local

```sh
# setup bitcoind
cd btc-deploy/ && docker-compose up --build -d && cd -
# deploy btc bridge
./scripts/deploy/btc-bridge.sh -m <create|install|reinstall|update>
```

When deploying on local, this will also deploy the ckbtc ledger and minter canisters.
By default this will run against the bitcoin regtest

### BTC NFT Bridge

Deploy the Bitcoin NFT bridge

```sh
./scripts/deploy/btc-nft-bridge.sh --help
```

```txt
Options:
  -h, --help                                      Display this help message
  -b, --bitcoin-network <network>                 Bitcoin network (regtest, testnet, mainnet) (default: regtest)
  -e, --evm-principal <principal>                 EVM Principal
  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)
  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)
  --ord-url <ord-url>                             URL of Ord service
```

If deploying on ic

```sh
./scripts/deploy/btc-nft-bridge.sh -m <create|install|reinstall|update> -i ic -e <evm-principal> --bitcoin-network mainnet --ord-url <url>
```

If deploying on local

```sh
# setup bitcoind
cd ord-testnet/ && docker-compose up --build -d && cd -
# deploy btc bridge
./scripts/deploy/btc-nft-bridge.sh -m <create|install|reinstall|update>
```

When deploying on local, this will also deploy the ckbtc ledger and minter canisters.
By default this will run against the bitcoin regtest

### ERC20 Bridge

Deploy the ERC20 Bridge

```sh
./scripts/deploy/erc20-bridge.sh --help
```

```txt
Options:
  -h, --help                                      Display this help message
  -e, --evm-principal <principal>                 EVM Principal
  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)
  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)
  --base-evm <canister-id>                        Base EVM link canister ID
  --wrapped-evm <canister-id>                     Wrapped EVM link canister ID
  --erc20-base-bridge-contract <canister-id>      ERC20 Base bridge contract canister ID
  --erc20-wrapped-bridge-contract <canister-id>   ERC20 Wrapped bridge contract canister ID
```

If deploying on ic

```sh
./scripts/deploy/erc20-bridge.sh
  -m <create|install|reinstall|update>
  -e <evm-principal>
  -i ic
  --base-evm <evm-principal-or-rpc-url>
  --wrapped-evm <evm-principal-or-rpc-url>
  --erc20-base-bridge-contract <erc20-base-eth-address>
  --erc20-wrapped-bridge-contract <erc20-wrapped-eth-address>
```

If deploying on local

```sh
./scripts/deploy-erc20-bridge.sh -m create
```

This will deploy the ERC20 bridge, setup the evm canister and the BFT bridge.

### ERC721 Bridge

```sh
./scripts/deploy/erc721-bridge.sh
```

```txt
Options:
  -h, --help                                      Display this help message
  -e, --evm-canister <principal>                  EVM canister principal
  -w, --wallet <ETH wallet address>               Ethereum wallet address for deploy
  --minter-address <minter-address>               Bridge minter address
  --dfx-setup                                     Setup dfx locally
```

In order to deploy the ERC721 bridge, run:

If deploying on ic

```sh
./scripts/deploy/erc721-bridge.sh -e <evm-principal> -m <minter-address> -w <wallet-address>
```

If deploying on localhost just pass the `--dfx-setup` option.

```sh
./scripts/deploy/erc721-bridge.sh --dfx-setup
```

### ICRC2 Minter

```sh
./scripts/deploy/icrc2-minter.sh
```

```txt
Options:
  -h, --help                                      Display this help message
  -e, --evm-principal <principal>                 EVM Principal
  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)
  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)
```

If deploying on ic

```sh
./scripts/deploy/icrc2-minter.sh -m <create|install|reinstall|update> -e <evm-principal> -i ic
```

If deploying on local

```sh
./scripts/deploy/icrc2-minter.sh -m <create|install|reinstall|update>
```

### Rune Bridge

```sh
./scripts/deploy/rune-bridge.sh
```

```txt
Options:
  -h, --help                                      Display this help message
  -b, --bitcoin-network <network>                 Bitcoin network (regtest, testnet, mainnet) (default: regtest)
  -e, --evm-principal <principal>                 EVM Principal
  -i, --ic-network <network>                      Internet Computer network (local, ic) (default: local)
  -m, --install-mode <mode>                       Install mode (create, init, reinstall, upgrade)
  --indexer-url <url>                             Indexer URL
```

If deploying on ic

```sh
./scripts/deploy/rune-bridge.sh
  -m <create|install|reinstall|update>
  -e <evm-principal> 
  -i ic 
  --bitcoin-network mainnet 
  --indexer-url <indexer-url>
```

If deploying on local

```sh
# setup bitcoind
cd btc-deploy/ && docker-compose up --build -d && cd -
# deploy rune bridge
./scripts/deploy/rune-bridge.sh -m <create|install|reinstall|update>
```

By default this will run against the bitcoin regtest
