// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import "src/WrappedToken.sol";
import "src/libraries/StringUtils.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";

abstract contract TokenManager is Initializable {
    using SafeERC20 for IERC20;

    // Indicates whether this contract is on the wrapped side
    bool internal _isWrappedSide;

    /// Mapping from `base token id` (can be anything, also ERC20 address) to Wrapped tokens ERC20 address
    mapping(bytes32 => address) internal _baseToWrapped;

    /// Mapping from Wrapped ERC20 token address to base token id (can be anything, also ERC20 address)
    mapping(address => bytes32) internal _wrappedToBase;

    /// List of wrapped tokens.
    address[] private _wrappedTokenList;

    /// Event for new wrapped token creation
    event WrappedTokenDeployedEvent(string name, string symbol, bytes32 baseTokenID, address wrappedERC20);

    /// Token metadata
    struct TokenMetadata {
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
    }

    function _initialize(bool isWrappedSide) internal initializer {
        _isWrappedSide = isWrappedSide;
    }

    /// @notice Checks if the contract is on the base side
    /// @return true if the contract is on the base side
    function isBaseSide() internal view returns (bool) {
        return !_isWrappedSide;
    }

    /// Creates a new ERC20 compatible token contract as a wrapper for the given `externalToken`.
    function deployERC20(string memory name, string memory symbol, bytes32 baseTokenID) public returns (address) {
        require(_isWrappedSide, "Only for wrapped side");
        require(_baseToWrapped[baseTokenID] == address(0), "Wrapper already exist");

        // Create the new token
        WrappedToken wrappedERC20 = new WrappedToken(name, symbol, address(this));

        _baseToWrapped[baseTokenID] = address(wrappedERC20);
        _wrappedToBase[address(wrappedERC20)] = baseTokenID;
        _wrappedTokenList.push(address(wrappedERC20));

        emit WrappedTokenDeployedEvent(name, symbol, baseTokenID, address(wrappedERC20));

        return address(wrappedERC20);
    }

    /// Update token's metadata
    function updateTokenMetadata(address token, bytes32 name, bytes16 symbol, uint8 decimals) internal {
        WrappedToken(token).setMetaData(name, symbol, decimals);
    }

    /// tries to query token metadata
    function getTokenMetadata(address token) internal view returns (TokenMetadata memory meta) {
        try IERC20Metadata(token).name() returns (string memory _name) {
            meta.name = StringUtils.truncateUTF8(_name);
        } catch {}
        try IERC20Metadata(token).symbol() returns (string memory _symbol) {
            meta.symbol = bytes16(StringUtils.truncateUTF8(_symbol));
        } catch {}
        try IERC20Metadata(token).decimals() returns (uint8 _decimals) {
            meta.decimals = _decimals;
        } catch {}
    }

    /// Returns wrapped token for the given base token
    function getWrappedToken(bytes32 baseTokenID) external view returns (address) {
        return _baseToWrapped[baseTokenID];
    }

    /// Returns base token for the given wrapped token
    function getBaseToken(address wrappedTokenAddress) external view returns (bytes32) {
        return _wrappedToBase[wrappedTokenAddress];
    }

    /// Returns list of token pairs.
    function listTokenPairs() external view returns (address[] memory wrapped, bytes32[] memory base) {
        uint256 length = _wrappedTokenList.length;
        wrapped = new address[](length);
        base = new bytes32[](length);
        for (uint256 i = 0; i < length; i++) {
            address wrappedToken = _wrappedTokenList[i];
            wrapped[i] = wrappedToken;
            base[i] = _wrappedToBase[wrappedToken];
        }
    }
}
