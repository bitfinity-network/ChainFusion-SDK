#!/usr/bin/env sh

set -e
set -x

export RUST_BACKTRACE=full

# Configuration variables
WASM_DIR=".artifact"

# This is the hash of a recent commit on the https://github.com/dfinity/ic repository.
# It is used to identify the IC canisters to download.
# To be updated periodically to use the latest version.
IC_COMMIT_HASH="c337458aafa8e8864eadfcfe38d7e5fa385b0006" # 1-11-2023
EVM_FEATURES="export-api"

# Function to print help instructions
print_help() {
    echo "Usage: $0 [all|evm|evm_testnet|signature_verification|spender|minter]"
    echo "Examples:"
    echo "  $0                      # Build all canisters, download binaries and build tools (default)"
    echo "  $0 all                  # Build all canisters and download binaries and build tools"
    echo "  $0 spender minter       # Build the spender and minter canisters"
}

# Initial setup
initialize_env() {
    [ -n "$ETHEREUM_GENESIS_ACCOUNTS" ] &&
        [ "$ETHEREUM_GENESIS_ACCOUNTS" -gt 0 ] &&
        EVM_FEATURES="$EVM_FEATURES,ethereum-genesis-accounts"

    echo "IC_HASH: $IC_HASH"

    if [ ! -f "./Cargo.toml" ]; then
        echo "Expecting to run from the cargo root directory, current directory is: $(pwd)"
        exit 42
    fi

    if [ "$CI" != "true" ]; then
        script_dir=$(dirname $0)
        project_dir=$(realpath "${script_dir}/..")

        echo "Project dir: \"$project_dir\""
        cd "$project_dir"

        rm -rf "$WASM_DIR"
        mkdir -p "$WASM_DIR"
    fi
}

# Function to download files
download_file() {
    local url="$1"
    local output_path="$2"
    echo "Downloading $url to $output_path"
    curl -o "$output_path" "$url"
}

get_icrc1_binaries() {
    download_file "https://download.dfinity.systems/ic/${IC_COMMIT_HASH}/canisters/ic-icrc1-ledger.wasm.gz" "$WASM_DIR/icrc1-ledger.wasm.gz"
    download_file "https://raw.githubusercontent.com/dfinity/ic/${IC_COMMIT_HASH}/rs/rosetta-api/icrc1/ledger/ledger.did" "$WASM_DIR/icrc1.did"
}

build_create_bft_bridge_tool() {
    echo "Building create BFTBridge tool"

    cargo build -p create_bft_bridge_tool --release
    cp target/release/create_bft_bridge_tool "$WASM_DIR/create_bft_bridge_tool"
}

# Function to build a single canister with a feature flag
build_canister() {
    local canister_name="$1"
    local features="$2"
    local output_wasm="$3"
    local did_file_name="${4:-$canister_name}"

    # Generate the did file
    cargo run -p "$canister_name" --features "$features" >"$WASM_DIR/$did_file_name.did"

    echo "Building $canister_name Canister with features: $features"

    cargo build --target wasm32-unknown-unknown --release --package "$canister_name" --features "$features"
    ic-wasm "target/wasm32-unknown-unknown/release/$canister_name.wasm" -o "$WASM_DIR/$output_wasm" shrink
    gzip -k "$WASM_DIR/$output_wasm" --force
}

# Function to determine which canisters to build based on input
build_requested_canisters() {
    if [ $# -eq 0 ]; then
        set -- "all"

    elif [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
        print_help
        exit 0
    fi

    if [ "$1" = "all" ]; then
        initialize_env
        # Download binaries only if "all" is specified
        echo "Getting ICRC-1 Binaries"
        get_icrc1_binaries

        # Build all canisters
        
        script_dir=$(dirname $0)
        project_dir=$(realpath "${script_dir}/..")

        build_canister "spender_canister" "export-api" "spender.wasm" "spender"
        build_canister "minter_canister" "export-api" "minter.wasm" "minter"
        build_canister "evm-minter" "export-api" "evm-minter.wasm" "evm-minter"

        # Build tools
        build_create_bft_bridge_tool
    else
        for canister in "$@"; do
            case "$canister" in
            spender | minter)
                build_canister "${canister}_canister" "export-api" "${canister}.wasm" "${canister}"
                ;;
            *)
                echo "Error: Unknown canister '$canister'."
                print_help
                exit 1
                ;;
            esac
        done
    fi
}

main() {
    build_requested_canisters "$@"
}

main "$@"
