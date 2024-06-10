// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "openzeppelin-contracts/utils/cryptography/ECDSA.sol";
import "openzeppelin-contracts/token/ERC20/utils/SafeERC20.sol";
import "src/WrappedToken.sol";
import "src/interfaces/IFeeCharge.sol";
import {RingBuffer} from "src/libraries/RingBuffer.sol";

contract BFTBridge {
    using RingBuffer for RingBuffer.RingBufferUint32;
    using SafeERC20 for IERC20;

    struct MintOrderData {
        uint256 amount;
        bytes32 senderID;
        bytes32 fromTokenID;
        address recipient;
        address toERC20;
        uint32 nonce;
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
        uint32 senderChainID;
        address approveSpender;
        uint256 approveAmount;
        address feePayer;
    }


    function truncateUTF8(
        string memory input
    ) public pure returns (bytes32 result) {
        // If the last byte starts with 0xxxxx, return the data as is
        bytes memory source = bytes(input);
        if (source.length < 32 || (source[31] & 0x80) == 0) {
            assembly {
                result := mload(add(source, 32))
            }
            return result;
        }

        if (source.length == 0) {
            return 0x0;
        }

        // Go backwards from the last byte until a byte that doesn't start with 10xxxxxx is found
        for (uint8 i = 31; i >= 0; i--) {
            if ((source[i] & 0xC0) != 0x80) {
                for (uint8 j = i; j < 32; j += 1) {
                    source[j] = 0;
                }

                assembly {
                    result := mload(add(source, 32))
                }

                break;
            }

            if (i == 0) {
                return 0x0;
            }
        }
    }

    // Additional gas amount for fee charge.
    // todo: estimate better: https://infinityswap.atlassian.net/browse/EPROD-919
    uint256 constant additionalGasFee = 1000;

    // Has a user's transaction nonce been used?
    mapping(bytes32 => mapping(uint32 => bool)) private _isNonceUsed;

    // Blocknumbers for users deposit Ids.
    mapping(address => mapping(uint8 => uint32)) private _userDepositBlocks;

    // Last 255 user's burn operations.
    mapping(address => RingBuffer.RingBufferUint32) private _lastUserBurns;

    // Get the wrapped token addresses given their native token.
    mapping(bytes32 => address) private _erc20TokenRegistry;

    // Mapping from Base tokens to Wrapped tokens
    mapping(address => bytes32) private _baseTokenRegistry;

    // List of wrapped tokens.
    address[] private _wrappedTokenList;

    // Address of minter canister
    address public minterCanisterAddress;

    // Address of feeCharge contract
    IFeeCharge public feeChargeContract;

    // Operataion ID counter
    uint32 public operationIDCounter;

    // Constructor to initialize minterCanisterAddress and feeChargeContract.
    constructor(address minterAddress, address feeChargeAddress) {
        minterCanisterAddress = minterAddress;
        feeChargeContract = IFeeCharge(feeChargeAddress);
    }

    // Event for mint operation
    event MintTokenEvent(
        uint256 amount,
        bytes32 fromToken,
        bytes32 senderID,
        address toERC20,
        address recipient,
        uint32 nonce
    );

    // Event for burn operation
    event BurnTokenEvent(
        address sender,
        uint256 amount,
        address fromERC20,
        bytes recipientID,
        bytes32 toToken,
        uint32 operationID,
        bytes32 name,
        bytes16 symbol,
        uint8 decimals
    );

    // Event for new wrapped token creation
    event WrappedTokenDeployedEvent(
        string name,
        string symbol,
        bytes32 baseTokenID,
        address wrappedERC20
    );

    // Struct with information about burn operation
    struct Erc20BurnInfo {
        address sender;
        uint256 amount;
        address fromERC20;
        bytes32 recipientID;
        bytes32 toToken;
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
    }

    // Event that can be emited with a notification for the minter canister
    event NotifyMinterEvent(
        uint32 notificationType,
        bytes userData
    );

    // Emit minter notification event with the given `userData`. For details about what should be in the user data,
    // check the implementation of the corresponding minter.
    function notifyMinter(uint32 notificationType, bytes calldata userData) external {
        emit NotifyMinterEvent(notificationType, userData);
    }

    // Main function to withdraw funds
    function mint(bytes calldata encodedOrder) external {
        uint256 initGasLeft = gasleft();

        MintOrderData memory order = _decodeAndValidateOrder(
            encodedOrder[: 269]
        );

        _checkMintOrderSignature(encodedOrder);

        // Cases:
        // 1. `_erc20TokenRegistry` contains the `order.fromTokenID`. So, we are in WrappedToken side.
        // 2. `_erc20TokenRegistry` does not contain the `order.fromTokenID`. So:
        //   a. We are in BaseToken side.
        //   b. We are minting NativeToken.
        address toToken = _erc20TokenRegistry[order.fromTokenID];

        // If we mint base token we don't have information about token pairs.
        // So, we need to get it from the order.
        if (toToken == address(0)) {
            toToken = order.toERC20;
        }

        // The toToken should be a valid token address.
        // This should never fail.
        require(
            toToken != address(0),
            "toToken address should be specified correctly"
        );

        // Update token's metadata
        WrappedToken(toToken).setMetaData(
            order.name,
            order.symbol,
            order.decimals
        );

        // Execute the withdrawal
        _isNonceUsed[order.senderID][order.nonce] = true;
        IERC20(toToken).safeTransfer(order.recipient, order.amount);

        
        if (order.approveSpender != address(0) && order.approveAmount != 0) {
            WrappedToken(toToken).approveByOwner(
                order.recipient,
                order.approveSpender,
                order.approveAmount
            );
        }

        if (order.feePayer != address(0) && msg.sender == minterCanisterAddress && address(feeChargeContract) != address(0)) {
            uint256 gasFee = initGasLeft - gasleft() + additionalGasFee;
            uint256 fee = gasFee * tx.gasprice;
            feeChargeContract.chargeFee(order.feePayer, payable(minterCanisterAddress), order.senderID, fee);
        }

        // Emit event
        emit MintTokenEvent(
            order.amount,
            order.fromTokenID,
            order.senderID,
            toToken,
            order.recipient,
            order.nonce
        );
    }

    // Getter function for block numbers
    function getDepositBlocks() external view returns (uint32[] memory blockNumbers) {
        blockNumbers = _lastUserBurns[msg.sender].getAll();
    }

    // Burn ERC 20 tokens there to make possible perform a mint on other side of the bridge.
    // Caller should approve transfer in the given `from_erc20` token for the bridge contract.
    // Returns operation ID if operation is succesfull.
    function burn(
        uint256 amount,
        address fromERC20,
        bytes memory recipientID
    ) public returns (uint32) {
        require(fromERC20 != address(this), "From address must not be BFT bridge address");

        IERC20(fromERC20).safeTransferFrom(msg.sender, address(this), amount);

        bytes32 toTokenID = _baseTokenRegistry[fromERC20];

        require(amount > 0, "Invalid burn amount");
        require(fromERC20 != address(0), "Invalid from address");

        // Update user information about burn operations.
        _lastUserBurns[msg.sender].push(uint32(block.number));

        // get the token details
        TokenMetadata memory meta = getTokenMetadata(fromERC20);

        uint32 operationID = operationIDCounter++;

        emit BurnTokenEvent(
            msg.sender,
            amount,
            fromERC20,
            recipientID,
            toTokenID,
            operationID,
            meta.name,
            meta.symbol,
            meta.decimals
        );

        return operationID;
    }

    struct TokenMetadata {
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
    }

    // tries to query token metadata
    function getTokenMetadata(address token) internal view returns (TokenMetadata memory meta) {
        try IERC20Metadata(token).name() returns (string memory _name) {
            meta.name = truncateUTF8(_name);
        } catch {}
        try IERC20Metadata(token).symbol() returns (
            string memory _symbol
        ) {
            meta.symbol = bytes16(truncateUTF8(_symbol));
        } catch {}
        try IERC20Metadata(token).decimals() returns (uint8 _decimals) {
            meta.decimals = _decimals;
        } catch {}
    }

    // Getter function for minter address
    function getMinterAddress() external view returns (address) {
        return minterCanisterAddress;
    }

    // Returns wrapped token for the given base token
    function getWrappedToken(
        bytes32 baseTokenID
    ) external view returns (address) {
        return _erc20TokenRegistry[baseTokenID];
    }

    // Returns base token for the given wrapped token
    function getBaseToken(
        address wrappedTokenAddress
    ) external view returns (bytes32) {
        return _baseTokenRegistry[wrappedTokenAddress];
    }

    // Returns list of token pairs.
    function listTokenPairs() external view returns (address[] memory wrapped, bytes32[] memory base) {
        uint length = _wrappedTokenList.length;
        wrapped = new address[](length);
        base = new bytes32[](length);
        for (uint i = 0; i < length; i++) {
            address wrappedToken = _wrappedTokenList[i];
            wrapped[i] = wrappedToken;
            base[i] = _baseTokenRegistry[wrappedToken];
        }
    }

    // Creates a new ERC20 compatible token contract as a wrapper for the given `externalToken`.
    function deployERC20(
        string memory name,
        string memory symbol,
        bytes32 baseTokenID
    ) public returns (address) {
        require(
            _erc20TokenRegistry[baseTokenID] == address(0),
            "Wrapper already exist"
        );

        // Create the new token
        WrappedToken wrappedERC20 = new WrappedToken(
            name,
            symbol,
            address(this)
        );

        _erc20TokenRegistry[baseTokenID] = address(wrappedERC20);
        _baseTokenRegistry[address(wrappedERC20)] = baseTokenID;
        _wrappedTokenList.push(address(wrappedERC20));

        emit WrappedTokenDeployedEvent(
            name,
            symbol,
            baseTokenID,
            address(wrappedERC20)
        );

        return address(wrappedERC20);
    }

    // Function to decode and validate the order data
    function _decodeAndValidateOrder(
        bytes calldata encodedOrder
    ) private view returns (MintOrderData memory order) {
        // Decode order data
        order.amount = uint256(bytes32(encodedOrder[: 32]));
        order.senderID = bytes32(encodedOrder[32 : 64]);
        order.fromTokenID = bytes32(encodedOrder[64 : 96]);
        order.recipient = address(bytes20(encodedOrder[96 : 116]));
        order.toERC20 = address(bytes20(encodedOrder[116 : 136]));
        order.nonce = uint32(bytes4(encodedOrder[136 : 140]));
        order.senderChainID = uint32(bytes4(encodedOrder[140 : 144]));
        uint32 recipientChainID = uint32(bytes4(encodedOrder[144 : 148]));
        order.name = bytes32(encodedOrder[148 : 180]);
        order.symbol = bytes16(encodedOrder[180 : 196]);
        order.decimals = uint8(encodedOrder[196]);
        order.approveSpender = address(bytes20(encodedOrder[197 : 217]));
        order.approveAmount = uint256(bytes32(encodedOrder[217 : 249]));
        order.feePayer = address(bytes20(encodedOrder[249 : 269]));

        // Assert recipient address is not zero
        require(order.recipient != address(0), "Invalid destination address");

        // Check if amount is greater than zero
        require(order.amount > 0, "Invalid order amount");

        // Check if nonce is not stored in the list
        require(!_isNonceUsed[order.senderID][order.nonce], "Invalid nonce");

        // Check if withdrawal is happening on the correct chain
        require(block.chainid == recipientChainID, "Invalid chain ID");

        if (_baseTokenRegistry[order.toERC20] != bytes32(0)) {
            require(
                _erc20TokenRegistry[order.fromTokenID] == order.toERC20,
                "SRC token and DST token must be a valid pair"
            );
        }
    }

    // Function to check encodedOrder signature
    function _checkMintOrderSignature(
        bytes calldata encodedOrder
    ) private view {
        // Create a hash of the order data
        bytes32 hash = keccak256(encodedOrder[: 269]);

        // Recover signer from the signature
        address signer = ECDSA.recover(hash, encodedOrder[269 :]);

        // Check if signer is the minter canister
        require(signer == minterCanisterAddress, "Invalid signature");
    }
}
