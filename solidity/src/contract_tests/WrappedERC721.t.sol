// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "src/WrappedERC721.sol";

contract WrappedERC721Test is Test {
    address _owner = makeAddr("owner");
    address _alice = makeAddr("alice");
    address _bob = makeAddr("bob");
    WrappedERC721 _token;

    function setUp() public {
        _token = new WrappedERC721("NFT", "TKN", _owner);
    }

    function testMint() public {
        assertEq(_token.balanceOf(_alice), 0);

        vm.prank(_owner);
        _token.safeMint(_alice, "uri");

        assertEq(_token.balanceOf(_alice), 1);
    }

    function testBurn() public {
        vm.prank(_owner);
        uint256 id = _token.safeMint(_alice, "uri");

        assertEq(_token.balanceOf(_alice), 1);

        vm.prank(_owner);
        _token.burn(id);
        assertEq(_token.balanceOf(_alice), 0);
    }

    function testSetMetadataSuccess() public {
        assertEq(_token.name(), "NFT");
        assertEq(_token.symbol(), "TKN");

        vm.prank(_owner);
        _token.setMetaData(
            bytes32(bytes("New token")),
            bytes16(bytes("New symbol"))
        );
        assertEq(
            _token.name(),
            string(abi.encodePacked(bytes32(bytes("New token"))))
        );
        assertEq(
            _token.symbol(),
            string(abi.encodePacked(bytes16(bytes("New symbol"))))
        );

        vm.prank(_owner);
        _token.setMetaData(0x0, 0x0);
        assertEq(
            _token.name(),
            string(abi.encodePacked(bytes32(bytes("New token"))))
        );
        assertEq(
            _token.symbol(),
            string(abi.encodePacked(bytes16(bytes("New symbol"))))
        );
    }

    function testSetMetadataInvalidCaller() public {
        vm.expectRevert();
        _token.setMetaData(
            bytes32(bytes("New token")),
            bytes16(bytes("New symbol"))
        );
    }
}
