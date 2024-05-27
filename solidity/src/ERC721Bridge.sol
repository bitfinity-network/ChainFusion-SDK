// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "openzeppelin-contracts/utils/cryptography/ECDSA.sol";
import "openzeppelin-contracts/token/ERC721/IERC721.sol";
import "openzeppelin-contracts/utils/Strings.sol";
import "src/WrappedERC721.sol";

contract ERC721Bridge {
    struct MintOrderData {
        bytes32 senderID;
        bytes32 fromTokenID;
        address recipient;
        address toERC721;
        uint32 nonce;
        bytes32 name;
        bytes16 symbol;
        uint32 senderChainID;
        address approveSpender;
        string tokenURI;
    }

    struct RingBuffer {
        uint8 begin;
        uint8 end;
    }

    function increment(
        RingBuffer memory buffer
    ) public pure returns (RingBuffer memory) {
        unchecked {
            buffer.end = buffer.end + 1;
        }
        if (buffer.begin == buffer.end) {
            unchecked {
                buffer.begin = buffer.begin + 1;
            }
        }
        return buffer;
    }

    // Function to get the size of the buffer
    function size(RingBuffer memory buffer) public pure returns (uint8) {
        uint8 sizeOf;
        if (buffer.begin <= buffer.end) {
            sizeOf = buffer.end - buffer.begin;
        } else {
            sizeOf = uint8(buffer.end + 256 - buffer.begin);
        }

        return sizeOf;
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

    function toIDfromBaseAddress(
        uint32 chainID,
        address toAddress
    ) public pure returns (bytes32 toID) {
        return
            bytes32(
                abi.encodePacked(
                    uint8(1),
                    chainID,
                    toAddress,
                    uint32(0),
                    uint16(0),
                    uint(8)
                )
            );
    }

    // Has a user's transaction nonce been used?
    mapping(bytes32 => mapping(uint32 => bool)) private _isNonceUsed;

    // Blocknumbers for users deposit Ids.
    mapping(address => mapping(uint8 => uint32)) private _userDepositBlocks;

    // Beginning and the end indices for the the user deposits
    mapping(address => RingBuffer) private _lastUserDeposit;

    // Get the wrapped token addresses given their native token.
    mapping(bytes32 => address) private _erc721TokenRegistry;

    // Mapping from Base tokens to Wrapped tokens
    mapping(address => bytes32) private _baseTokenRegistry;

    // Address of minter canister
    address public minterCanisterAddress;

    // Operataion ID counter
    uint32 public operationIDCounter;

    // Constructor to initialize minterCanisterAddress
    constructor(address minterAddress) {
        minterCanisterAddress = minterAddress;
    }

    // Event for mint operation
    event MintTokenEvent(
        bytes32 fromToken,
        bytes32 senderID,
        address toERC721,
        address recipient,
        uint256 tokenId,
        uint32 nonce
    );

    // Event for burn operation
    event BurnTokenEvent(
        address sender,
        address fromERC721,
        bytes recipientID,
        bytes32 toToken,
        uint32 operationID,
        bytes32 name,
        bytes16 symbol,
        string tokenURI
    );

    // Event for new wrapped token creation
    event WrappedTokenDeployedEvent(
        string name,
        string symbol,
        bytes32 baseTokenID,
        address wrappedERC721
    );

    // Struct with information about burn operation
    struct Erc721BurnInfo {
        address sender;
        address fromERC721;
        bytes32 recipientID;
        bytes32 toToken;
        bytes32 name;
        bytes16 symbol;
    }

    // Main function to withdraw funds
    function mint(bytes calldata encodedOrder) external {
        MintOrderData memory order = _decodeAndValidateClaim(encodedOrder);

        _checkMintOrderSignature(encodedOrder);

        // Cases:
        // 1. `_erc721TokenRegistry` contains the `order.fromTokenID`. So, we are in WrappedERC721 side.
        // 2. `_erc721TokenRegistry` does not contain the `order.fromTokenID`. So:
        //   a. We are in BaseToken side.
        //   b. We are minting NativeToken.
        address toToken = _erc721TokenRegistry[order.fromTokenID];

        // If we mint base token we don't have information about token pairs.
        // So, we need to get it from the order.
        if (toToken == address(0)) {
            toToken = order.toERC721;
        }

        // The toToken should be a valid token address.
        // This should never fail.
        require(
            toToken != address(0),
            "toToken address should be specified correctly"
        );

        // Update token's metadata
        WrappedERC721(toToken).setMetaData(order.name, order.symbol);

        // Execute the withdrawal
        _isNonceUsed[order.senderID][order.nonce] = true;
        uint256 tokenId = WrappedERC721(toToken).safeMint(
            order.recipient,
            order.tokenURI
        );

        if (order.approveSpender != address(0)) {
            WrappedERC721(toToken).approveByOwner(
                order.recipient,
                order.approveSpender,
                tokenId
            );
        }

        // Emit event
        emit MintTokenEvent(
            order.fromTokenID,
            order.senderID,
            toToken,
            order.recipient,
            tokenId,
            order.nonce
        );
    }

    // Getter function for block numbers
    function getDepositBlocks() external view returns (uint32[] memory) {
        RingBuffer memory buffer = _lastUserDeposit[msg.sender];

        // Get buffer size
        uint8 bufferSize = size(buffer);

        // Fill return buffer with values
        uint32[] memory res = new uint32[](bufferSize);
        for (uint8 i = buffer.begin; i != buffer.end; ) {
            res[i] = _userDepositBlocks[msg.sender][i]; // Assign values to the temporary array
            unchecked {
                i++;
            }
        }

        return res;
    }

    // Burn ERC 20 tokens there to make possible perform a mint on other side of the bridge.
    // Caller should approve transfer in the given `from_erc20` token for the bridge contract.
    // Returns operation ID if operation is succesfull.
    function burn(
        uint256 tokenID,
        address fromERC721,
        bytes memory recipientID
    ) public returns (uint32) {
        require(
            fromERC721 != address(this),
            "From address must not be BFT bridge address"
        );

        IERC721(fromERC721).safeTransferFrom(
            msg.sender,
            address(this),
            tokenID
        );

        bytes32 toTokenID = _baseTokenRegistry[fromERC721];
        string memory tokenURI = WrappedERC721(fromERC721).tokenURI(tokenID);
        require(fromERC721 != address(0), "Invalid from address");

        // Update user information about burn operations.
        // Additional scope required to save stack space and avoid StackTooDeep error.
        {
            RingBuffer memory buffer = _lastUserDeposit[msg.sender];
            _userDepositBlocks[msg.sender][buffer.end] = uint32(block.number);
            _lastUserDeposit[msg.sender] = increment(buffer);
        }

        // get the token details
        TokenMetadata memory meta = getTokenMetadata(fromERC721);

        uint32 operationID = operationIDCounter++;

        emit BurnTokenEvent(
            msg.sender,
            fromERC721,
            recipientID,
            toTokenID,
            operationID,
            meta.name,
            meta.symbol,
            tokenURI
        );

        return operationID;
    }

    struct TokenMetadata {
        bytes32 name;
        bytes16 symbol;
    }

    // tries to query token metadata
    function getTokenMetadata(
        address token
    ) internal view returns (TokenMetadata memory meta) {
        try WrappedERC721(token).name() returns (string memory _name) {
            meta.name = truncateUTF8(_name);
        } catch {}
        try WrappedERC721(token).symbol() returns (string memory _symbol) {
            meta.symbol = bytes16(truncateUTF8(_symbol));
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
        return _erc721TokenRegistry[baseTokenID];
    }

    // Returns base token for the given wrapped token
    function getBaseToken(
        address wrappedTokenAddress
    ) external view returns (bytes32) {
        return _baseTokenRegistry[wrappedTokenAddress];
    }

    // Creates a new ERC721 compatible token contract as a wrapper for the given `externalToken`.
    function deployERC721(
        string memory name,
        string memory symbol,
        bytes32 baseTokenID
    ) public returns (address) {
        require(
            _erc721TokenRegistry[baseTokenID] == address(0),
            "Wrapper already exist"
        );

        // Create the new token
        WrappedERC721 wrappedERC721 = new WrappedERC721(
            name,
            symbol,
            address(this)
        );

        _erc721TokenRegistry[baseTokenID] = address(wrappedERC721);
        _baseTokenRegistry[address(wrappedERC721)] = baseTokenID;

        emit WrappedTokenDeployedEvent(
            name,
            symbol,
            baseTokenID,
            address(wrappedERC721)
        );

        return address(wrappedERC721);
    }

    // Function to decode and validate the order data
    function _decodeAndValidateClaim(
        bytes calldata encodedOrder
    ) private view returns (MintOrderData memory) {
        // Decode order data
        bytes32 senderID = bytes32(encodedOrder[:32]);
        bytes32 fromTokenID = bytes32(encodedOrder[32:64]);
        address recipient = address(bytes20(encodedOrder[64:84]));
        address toERC721 = address(bytes20(encodedOrder[84:104]));
        uint32 nonce = uint32(bytes4(encodedOrder[104:108]));
        uint32 senderChainId = uint32(bytes4(encodedOrder[108:112]));
        uint32 recipientChainId = uint32(bytes4(encodedOrder[112:116]));
        bytes32 name = bytes32(encodedOrder[116:148]);
        bytes16 symbol = bytes16(encodedOrder[148:164]);
        address approveSpender = address(bytes20(encodedOrder[164:184]));
        uint32 dataSize = uint32(bytes4(encodedOrder[184:188]));
        bytes memory data = encodedOrder[188:188 + dataSize];
        string memory tokenUri = string(abi.encodePacked(data));

        // Assert recipient address is not zero
        require(recipient != address(0), "Invalid destination address");

        // Check if nonce is not stored in the list
        require(!_isNonceUsed[senderID][nonce], "Invalid nonce");

        // Check if withdrawal is happening on the correct chain
        require(block.chainid == recipientChainId, "Invalid chain ID");

        if (_baseTokenRegistry[toERC721] != bytes32(0)) {
            require(
                _erc721TokenRegistry[fromTokenID] == toERC721,
                "SRC token and DST token must be a valid pair"
            );
        }

        // Return the decoded order data
        return
            MintOrderData(
                senderID,
                fromTokenID,
                recipient,
                toERC721,
                nonce,
                name,
                symbol,
                senderChainId,
                approveSpender,
                tokenUri
            );
    }

    // Function to check encodedOrder signature
    function _checkMintOrderSignature(
        bytes calldata encodedOrder
    ) private view {
        // Create a hash of the order data
        uint32 dataSize = uint32(bytes4(encodedOrder[184:188]));

        bytes32 hash = keccak256(encodedOrder[:188 + dataSize]);

        // Recover signer from the signature
        address signer = ECDSA.recover(hash, encodedOrder[188 + dataSize:]);

        // Check if signer is the minter canister
        require(signer == minterCanisterAddress, "Invalid signature");
    }
}
