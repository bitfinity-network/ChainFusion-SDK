// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "forge-std/console.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "src/BftBridge.sol";
import "src/test_contracts/UUPSProxy.sol";
import "src/WrappedToken.sol";
import "src/libraries/StringUtils.sol";
import "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {Upgrades} from "@openzeppelin-foundry-upgrades/Upgrades.sol";
import {Options} from "@openzeppelin-foundry-upgrades/Options.sol";

contract BftBridgeTest is Test {

    using StringUtils for string;

    struct MintOrder {
        uint256 amount;
        bytes32 senderID;
        bytes32 fromTokenID;
        address recipient;
        address toERC20;
        uint32 nonce;
        uint32 senderChainID;
        uint32 recipientChainID;
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
        address approveSpender;
        uint256 approveAmount;
        address feePayer;
    }

    uint256 constant _OWNER_KEY = 1;
    uint256 constant _ALICE_KEY = 2;
    uint256 constant _BOB_KEY = 3;

    uint32 constant _CHAIN_ID = 31555;

    address _owner = vm.addr(_OWNER_KEY);
    address _alice = vm.addr(_ALICE_KEY);
    address _bob = vm.addr(_BOB_KEY);

    BFTBridge _wrappedBridge;
    BFTBridge _baseBridge;

    address newImplementation = address(8);

    address wrappedProxy;
    address baseProxy;

    function setUp() public {
        vm.chainId(_CHAIN_ID);
        vm.startPrank(_owner);

        // Encode the initialization call
        bytes memory initializeData = abi.encodeWithSelector(
            BFTBridge.initialize.selector,
            _owner,
            address(0),
            true
        );
        Options memory opts;
        // Skips all upgrade safety checks
        opts.unsafeSkipAllChecks = true;

        wrappedProxy = Upgrades.deployUUPSProxy(
            "BftBridge.sol:BFTBridge",
            initializeData,
            opts
        );

        // Cast the proxy to BFTBridge
        _wrappedBridge = BFTBridge(address(wrappedProxy));

        // Encode the initialization call
        bytes memory baseInitializeData = abi.encodeWithSelector(
            BFTBridge.initialize.selector,
            _owner,
            address(0),
            false
        );
        Options memory baseOpts;
        // Skips all upgrade safety checks
        baseOpts.unsafeSkipAllChecks = true;

        baseProxy = Upgrades.deployUUPSProxy(
            "BftBridge.sol:BFTBridge",
            baseInitializeData,
            baseOpts
        );

        // Cast the proxy to BFTBridge
        _baseBridge = BFTBridge(address(baseProxy));

        vm.stopPrank();
    }

    function testMinterCanisterAddress() public view {
        assertEq(_wrappedBridge.minterCanisterAddress(), _owner);
    }

    function testMintERC20FromICRC2Success() public {
        MintOrder memory order = _createDefaultMintOrder();
        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        _wrappedBridge.mint(encodedOrder);

        assertEq(
            WrappedToken(order.toERC20).balanceOf(order.recipient),
            order.amount
        );
    }

    function testMintERC20FromICRC2InvalidChainID() public {
        MintOrder memory order = _createDefaultMintOrder();
        order.recipientChainID = 31000;

        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.expectRevert(bytes("Invalid chain ID"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2InvalidRecipient() public {
        MintOrder memory order = _createDefaultMintOrder();
        order.recipient = address(0);

        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);
        vm.expectRevert(bytes("Invalid destination address"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2InvalidAmount() public {
        MintOrder memory order = _createDefaultMintOrder();
        order.amount = 0;

        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.expectRevert(bytes("Invalid order amount"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2UsedNonce() public {
        MintOrder memory order = _createDefaultMintOrder();
        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        _wrappedBridge.mint(encodedOrder);

        order.amount = 200;
        order.recipient = _bob;
        encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.expectRevert(bytes("Invalid nonce"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2InvalidPair() public {
        MintOrder memory order = _createDefaultMintOrder();
        order.fromTokenID = _createIdFromPrincipal(abi.encodePacked(uint8(1)));

        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.expectRevert(bytes("SRC token and DST token must be a valid pair"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2InvalidSignature() public {
        MintOrder memory order = _createDefaultMintOrder();

        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);
        // make signature corrupted
        encodedOrder[0] = bytes1(uint8(42));

        vm.expectRevert(bytes("Invalid signature"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintERC20FromICRC2InvalidOrderLength() public {
        bytes memory encodedOrder = abi.encodePacked(
            uint8(1),
            uint8(2),
            uint8(3),
            uint8(4)
        );

        vm.expectRevert();
        _wrappedBridge.mint(encodedOrder);
    }

    function testGetWrappedToken() public {
        bytes32 base_token_id = _createIdFromPrincipal(
            abi.encodePacked(uint8(1))
        );
        address wrapped_address = _wrappedBridge.deployERC20(
            "Token",
            "TKN",
            base_token_id
        );
        assertEq(wrapped_address, _wrappedBridge.getWrappedToken(base_token_id));
    }

    function testGetBaseToken() public {
        bytes32 base_token_id = _createIdFromPrincipal(
            abi.encodePacked(uint8(1))
        );
        address wrapped_address = _wrappedBridge.deployERC20(
            "Token",
            "TKN",
            base_token_id
        );
        assertEq(base_token_id, _wrappedBridge.getBaseToken(wrapped_address));
    }

    function testListTokenPairs() public {
        bytes32[3] memory base_token_ids = [
            _createIdFromPrincipal(abi.encodePacked(uint8(1))),
            _createIdFromPrincipal(abi.encodePacked(uint8(2))),
            _createIdFromPrincipal(abi.encodePacked(uint8(3)))
        ];

        address[3] memory wrapped_tokens;
        for (uint256 i = 0; i < 3; i++) {
            address wrapped_address = _wrappedBridge.deployERC20(
                "Token",
                "TKN",
                base_token_ids[i]
            );
            wrapped_tokens[i] = wrapped_address;
        }

        (address[] memory wrapped, bytes32[] memory base) = _wrappedBridge
            .listTokenPairs();

        for (uint256 i = 0; i < 3; i++) {
            assertEq(wrapped[i], wrapped_tokens[i]);
            assertEq(base[i], base_token_ids[i]);
        }
    }

    function testBurnWrappedSideWithDeployedErc20() public {
        bytes memory principal = abi.encodePacked(uint8(1), uint8(2), uint8(3));

        // deploy erc20 so it can be used
        MintOrder memory order = _createSelfMintOrder();
        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.prank(address(_owner));
        IERC20(order.toERC20).approve(address(_wrappedBridge), 1000);
        _wrappedBridge.mint(encodedOrder);

        assertEq(
            WrappedToken(order.toERC20).balanceOf(address(_owner)),
            order.amount
        );

        vm.prank(address(_owner));
        _wrappedBridge.burn(1, order.toERC20, order.fromTokenID, principal);
    }

    function testBurnWrappedSideWithUnregisteredToken() public {
        bytes memory principal = abi.encodePacked(uint8(1), uint8(2), uint8(3));

        address erc20 = address(new WrappedToken("omar", "OMAR", _owner));

        bytes32 toTokenId = _createIdFromPrincipal(abi.encodePacked(uint8(1)));
        vm.expectRevert(
            bytes("Invalid from address; not registered in the bridge")
        );
        _wrappedBridge.burn(100, erc20, toTokenId, principal);
    }

    function testBurnBaseSideWithUnregisteredToken() public {
        bytes memory principal = abi.encodePacked(uint8(1), uint8(2), uint8(3));

        WrappedToken erc20 = new WrappedToken("omar", "OMAR", _owner);
        address erc20Address = address(erc20);

        vm.prank(address(_owner));
        erc20.transfer(address(_owner), 100);
        vm.prank(address(_owner));
        erc20.approve(address(_baseBridge), 100);

        bytes32 toTokenId = _createIdFromPrincipal(abi.encodePacked(uint8(1)));
        vm.prank(address(_owner));
        _baseBridge.burn(100, erc20Address, toTokenId, principal);
    }

    function testMintBaseSideWithUnregisteredToken() public {
        WrappedToken erc20 = new WrappedToken("omar", "OMAR", _owner);
        address erc20Address = address(erc20);

        vm.prank(address(_owner));
        erc20.transfer(address(_baseBridge), 1000);

        MintOrder memory order = _createMintOrder(_alice, erc20Address);
        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        _baseBridge.mint(encodedOrder);

        assertEq(
            erc20.balanceOf(order.recipient),
            order.amount
        );
    }

    function testMintWrappedSideWithUnregisteredToken() public {
        WrappedToken erc20 = new WrappedToken("omar", "OMAR", _owner);
        address erc20Address = address(erc20);

        vm.prank(address(_owner));
        erc20.transfer(address(_wrappedBridge), 1000);

        MintOrder memory order = _createMintOrder(_alice, erc20Address);
        bytes memory encodedOrder = _encodeMintOrder(order, _OWNER_KEY);

        vm.expectRevert(bytes("Invalid token pair"));
        _wrappedBridge.mint(encodedOrder);
    }

    function testMintCallsAreRejectedWhenPaused() public {
        vm.prank(_owner);

        _wrappedBridge.pause();

        MintOrder memory mintOrder = _createDefaultMintOrder();
        vm.expectRevert(abi.encodeWithSignature("EnforcedPause()"));
        _wrappedBridge.mint(_encodeMintOrder(mintOrder, _OWNER_KEY));

        vm.prank(_owner);
        _wrappedBridge.unpause();

        // mint will be success
        _wrappedBridge.mint(_encodeMintOrder(mintOrder, _OWNER_KEY));
    }

    function testAddAllowedImplementation() public {
        vm.startPrank(_owner);

        BFTBridge _newImpl = new BFTBridge();

        newImplementation = address(_newImpl);

        _wrappedBridge.addAllowedImplementation(newImplementation);

        assertTrue(_wrappedBridge.allowedImplementations(newImplementation.codehash));

        vm.stopPrank();
    }

    function testAddAllowedImplementationOnlyOwner() public {
        vm.prank(address(10));

        vm.expectRevert();

        _wrappedBridge.addAllowedImplementation(newImplementation);
    }

    function testAddAllowedImplementationEmptyAddress() public {
        vm.prank(_owner);
        newImplementation = address(0);

        vm.expectRevert();

        _wrappedBridge.addAllowedImplementation(newImplementation);
    }

    /// Test that the bridge can be upgraded to a new implementation
    /// and the new implementation has been added to the list of allowed
    /// implementations
    function testUpgradeBridgeWithAllowedImplementation() public {
        vm.startPrank(_owner);

        BFTBridge _newImpl = new BFTBridge();

        newImplementation = address(_newImpl);

        _wrappedBridge.addAllowedImplementation(newImplementation);
        assertTrue(_wrappedBridge.allowedImplementations(newImplementation.codehash));

        // Wrap in ABI for easier testing
        BFTBridge proxy = BFTBridge(wrappedProxy);

        // pass empty calldata to initialize
        bytes memory data = new bytes(0);

        proxy.upgradeToAndCall(address(_newImpl), data);

        vm.stopPrank();
    }

    function testUpgradeBridgeWithNotAllowedImplementation() public {
        vm.startPrank(_owner);
        BFTBridge _newImpl = new BFTBridge();
        newImplementation = address(_newImpl);
        // Wrap in ABI for easier testing

        BFTBridge proxy = BFTBridge(wrappedProxy);
        // pass empty calldata to initialize
        bytes memory data = new bytes(0);
        vm.expectRevert();
        proxy.upgradeToAndCall(address(_newImpl), data);

        vm.stopPrank();
    }

    struct ExpectedBurnEvent {
        address sender;
        uint256 amount;
        address fromERC20;
        bytes32 recipientID;
        bytes32 toToken;
        bytes32 name;
        bytes16 symbol;
        uint8 decimals;
    }

    function _expectBurnEvent(ExpectedBurnEvent memory expected) private {
        Vm.Log[] memory entries = vm.getRecordedLogs();

        bool eventFound = false;

        for (uint256 i = 0; i < entries.length; i += 1) {
            if (
                entries[i].topics[0] ==
                keccak256(
                    "BurnTokenEvent(address,uint256,address,bytes32,bytes32,bytes32,bytes16,uint8)"
                )
            ) {
                assertEq(eventFound, false);
                eventFound = true;

                assertEq(entries[i].emitter, address(_wrappedBridge));

                assertEq(entries[i].topics.length, 1);

                (
                    address sender,
                    uint256 amount,
                    address fromERC20,
                    bytes32 recipientID,
                    bytes32 toToken,
                    bytes32 name,
                    bytes16 symbol,
                    uint8 decimals
                ) = abi.decode(
                        entries[i].data,
                        (
                            address,
                            uint256,
                            address,
                            bytes32,
                            bytes32,
                            bytes32,
                            bytes16,
                            uint8
                        )
                    );
                assertEq(expected.sender, sender);
                assertEq(expected.amount, amount);
                assertEq(expected.fromERC20, fromERC20);
                assertEq(expected.recipientID, recipientID);
                assertEq(expected.toToken, toToken);
                assertEq(expected.name, name);
                assertEq(expected.symbol, symbol);
                assertEq(expected.decimals, decimals);
            }
        }

        assertEq(eventFound, true);
    }

    function _createDefaultMintOrder()
        private
        returns (MintOrder memory order)
    {
        order.amount = 1000;
        order.senderID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3))
        );
        order.fromTokenID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3), uint8(4))
        );
        order.recipient = _alice;
        order.toERC20 = _wrappedBridge.deployERC20("Token", "TKN", order.fromTokenID);
        order.nonce = 0;
        order.senderChainID = 0;
        order.recipientChainID = _CHAIN_ID;
        // order.name = _bridge.truncateUTF8("Token");
        order.name = StringUtils.truncateUTF8("Token");
        // order.symbol = bytes16(_bridge.truncateUTF8("Token"));
        order.symbol = bytes16(StringUtils.truncateUTF8("Token"));
        order.decimals = 18;
        order.approveSpender = address(0);
        order.approveAmount = 0;
        order.feePayer = address(0);
    }

    function _createSelfMintOrder() private returns (MintOrder memory order) {
        order.amount = 1000;
        order.senderID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3))
        );
        order.fromTokenID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3), uint8(4))
        );
        order.recipient = address(_owner);
        order.toERC20 = _wrappedBridge.deployERC20("Token", "TKN", order.fromTokenID);
        order.nonce = 0;
        order.senderChainID = 0;
        order.recipientChainID = _CHAIN_ID;
        // order.name = _bridge.truncateUTF8("Token");
        order.name = StringUtils.truncateUTF8("Token");
        // order.symbol = bytes16(_bridge.truncateUTF8("Token"));
        order.symbol = bytes16(StringUtils.truncateUTF8("Token"));
        order.decimals = 18;
        order.approveSpender = address(0);
        order.approveAmount = 0;
        order.feePayer = address(0);
    }

    function _createMintOrder(address recipient, address toERC20) pure private returns (MintOrder memory order) {
        order.amount = 1000;
        order.senderID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3))
        );
        order.fromTokenID = _createIdFromPrincipal(
            abi.encodePacked(uint8(1), uint8(2), uint8(3), uint8(4))
        );
        order.recipient = recipient;
        order.toERC20 = toERC20;
        order.nonce = 0;
        order.senderChainID = 0;
        order.recipientChainID = _CHAIN_ID;
        // order.name = _bridge.truncateUTF8("Token");
        order.name = StringUtils.truncateUTF8("Token");
        // order.symbol = bytes16(_bridge.truncateUTF8("Token"));
        order.symbol = bytes16(StringUtils.truncateUTF8("Token"));
        order.decimals = 18;
        order.approveSpender = address(0);
        order.approveAmount = 0;
        order.feePayer = address(0);
    }

    function _encodeMintOrder(
        MintOrder memory order,
        uint256 privateKey
    ) private pure returns (bytes memory) {
        // Encoding splitted in two parts to avoid problems with stack overflow.
        bytes memory encodedOrder = abi.encodePacked(
            order.amount,
            order.senderID,
            order.fromTokenID,
            order.recipient,
            order.toERC20,
            order.nonce,
            order.senderChainID,
            order.recipientChainID,
            order.name,
            order.symbol,
            order.decimals,
            order.approveSpender,
            order.approveAmount,
            address(0)
        );
        bytes32 hash = keccak256(encodedOrder);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, hash);

        return abi.encodePacked(encodedOrder, r, s, v);
    }

    function _createIdFromPrincipal(
        bytes memory principal
    ) private pure returns (bytes32) {
        return
            bytes32(
                abi.encodePacked(uint8(0), uint8(principal.length), principal)
            );
    }

    function _createIdFromAddress(
        address addr,
        uint32 chainID
    ) private pure returns (bytes32) {
        return bytes32(abi.encodePacked(uint8(1), chainID, addr));
    }

}
