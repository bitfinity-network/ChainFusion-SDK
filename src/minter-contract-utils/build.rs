use std::collections::HashMap;
use std::io::ErrorKind;

use solidity_helper::error::SolidityHelperError;
use solidity_helper::{compile_solidity_contracts, SolidityContract};

fn main() -> anyhow::Result<()> {
    let contracts = match compile_solidity_contracts(None, None) {
        Ok(c) => c,
        Err(SolidityHelperError::IoError(err)) if err.kind() == ErrorKind::NotFound => {
            return Err(anyhow::anyhow!(
                "`forge` executable not found. Try installing forge with foundry: https://book.getfoundry.sh/getting-started/installation or check if it is present in the PATH"
            ))
        }
        Err(err) => {
            return Err(anyhow::anyhow!("Failed to compile solidity contracts: {err:?}"))
        }
    };

    set_contract_code(
        &contracts,
        "WrappedToken",
        "BUILD_SMART_CONTRACT_WRAPPED_TOKEN_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "WrappedERC721",
        "BUILD_SMART_CONTRACT_WRAPPED_ERC721_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "BFTBridge",
        "BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "ERC721Bridge",
        "BUILD_SMART_CONTRACT_ERC721_BRIDGE_HEX_CODE",
    );
    set_deployed_contract_code(
        &contracts,
        "BFTBridge",
        "BUILD_SMART_CONTRACT_BFT_BRIDGE_DEPLOYED_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "UniswapV2Factory",
        "BUILD_SMART_CONTRACT_UNISWAP_FACTORY_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "UniswapV2ERC20",
        "BUILD_SMART_CONTRACT_UNISWAP_TOKEN_HEX_CODE",
    );
    set_contract_code(
        &contracts,
        "WatermelonToken",
        "BUILD_SMART_CONTRACT_TEST_WTM_HEX_CODE",
    );

    Ok(())
}

/// Loads the contract with the specified name
fn set_contract_code(
    contracts: &HashMap<String, SolidityContract>,
    contract_name: &str,
    env_var: &str,
) {
    let contract_hex = &get_solidity_contract(contracts, contract_name).bytecode_hex;

    set_var(env_var, contract_hex);
}

/// Loads the deployed contract bytecode with the specified name
fn set_deployed_contract_code(
    contracts: &HashMap<String, SolidityContract>,
    contract_name: &str,
    env_var: &str,
) {
    let deployed_contract_hex =
        &get_solidity_contract(contracts, contract_name).deployed_bytecode_hex;

    set_var(env_var, deployed_contract_hex);
}

fn get_solidity_contract<'a>(
    contracts: &'a HashMap<String, SolidityContract>,
    contract_name: &str,
) -> &'a SolidityContract {
    contracts
        .get(contract_name)
        .unwrap_or_else(|| panic!("Cannot find the {contract_name} contract"))
}

// this sets a compile time variable
fn set_var(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}
