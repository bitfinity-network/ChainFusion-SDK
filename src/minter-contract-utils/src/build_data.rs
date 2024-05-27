use once_cell::sync::Lazy;

/// Bridge contract deployed bytecode
const BUILD_SMART_CONTRACT_BFT_BRIDGE_DEPLOYED_HEX_CODE: &str =
    env!("BUILD_SMART_CONTRACT_BFT_BRIDGE_DEPLOYED_HEX_CODE");

/// Get contract code from the environment variable
fn get_contract_code(env_name: &str) -> Vec<u8> {
    hex::decode(env_name)
        .unwrap_or_else(|_| panic!("failed to decode smart contract bytecode from '{env_name}'"))
}

/// BftBridge smart contract deployed bytecode
pub static BFT_BRIDGE_SMART_CONTRACT_DEPLOYED_CODE: Lazy<Vec<u8>> =
    Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_BFT_BRIDGE_DEPLOYED_HEX_CODE));

/// Bridge contract bytecode
const BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE: &str =
    env!("BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE");

/// BftBridge smart contract bytecode
pub static BFT_BRIDGE_SMART_CONTRACT_CODE: Lazy<Vec<u8>> =
    Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE));

#[cfg(feature = "test-contracts")]
pub mod test_contracts {
    use once_cell::sync::Lazy;

    use super::get_contract_code;

    /// Wrapped token contract bytecode
    const BUILD_SMART_CONTRACT_WRAPPED_TOKEN_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_WRAPPED_TOKEN_HEX_CODE");

    /// Wrapped token contract bytecode
    const BUILD_SMART_CONTRACT_WRAPPED_ERC721_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_WRAPPED_ERC721_HEX_CODE");

    /// Bridge contract bytecode
    const BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE");

    /// Bridge contract bytecode
    const BUILD_SMART_CONTRACT_ERC721_BRIDGE_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_ERC721_BRIDGE_HEX_CODE");

    /// Uniswap factory bytecode
    const BUILD_SMART_CONTRACT_UNISWAP_FACTORY_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_UNISWAP_FACTORY_HEX_CODE");

    const BUILD_SMART_CONTRACT_UNISWAP_TOKEN_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_UNISWAP_TOKEN_HEX_CODE");

    const BUILD_SMART_CONTRACT_TEST_WTM_HEX_CODE: &str =
        env!("BUILD_SMART_CONTRACT_TEST_WTM_HEX_CODE");

    /// WrappedToken smart contract bytecode
    pub static WRAPPED_TOKEN_SMART_CONTRACT_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_WRAPPED_TOKEN_HEX_CODE));

    /// WrappedERC721 smart contract bytecode
    pub static WRAPPED_ERC721_SMART_CONTRACT_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_WRAPPED_ERC721_HEX_CODE));

    /// BftBridge smart contract bytecode
    pub static BFT_BRIDGE_SMART_CONTRACT_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_BFT_BRIDGE_HEX_CODE));

    /// ERC721Bridge smart contract bytecode
    pub static ERC721_BRIDGE_SMART_CONTRACT_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_ERC721_BRIDGE_HEX_CODE));

    /// Uniswap factory contract bytecode
    pub static UNISWAP_FACTORY_HEX_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_UNISWAP_FACTORY_HEX_CODE));

    /// Uniswap token contract bytecode
    pub static UNISWAP_TOKEN_HEX_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_UNISWAP_TOKEN_HEX_CODE));

    /// Uniswap token contract bytecode
    pub static TEST_WTM_HEX_CODE: Lazy<Vec<u8>> =
        Lazy::new(|| get_contract_code(BUILD_SMART_CONTRACT_TEST_WTM_HEX_CODE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_get_wrapped_erc721_smart_contract_code() {
        let code = &*test_contracts::WRAPPED_ERC721_SMART_CONTRACT_CODE;
        assert!(!code.is_empty())
    }

    #[test]
    fn should_get_erc721_bridge_smart_contract_code() {
        let code = &*test_contracts::ERC721_BRIDGE_SMART_CONTRACT_CODE;
        assert!(!code.is_empty())
    }

    #[test]
    fn should_get_wrapped_token_smart_contract_code() {
        let code = &*test_contracts::WRAPPED_TOKEN_SMART_CONTRACT_CODE;
        assert!(!code.is_empty())
    }

    #[test]
    fn should_get_bft_bridge_token_smart_contract_code() {
        let code = &*BFT_BRIDGE_SMART_CONTRACT_CODE;
        assert!(!code.is_empty())
    }

    #[test]
    fn should_get_bft_bridge_token_smart_contract_deployed_code() {
        let code = &*BFT_BRIDGE_SMART_CONTRACT_DEPLOYED_CODE;
        assert!(!code.is_empty())
    }
}
