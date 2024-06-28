// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "src/WrappedToken.sol";

contract WrappedTokenTest is Test {
    address _owner = makeAddr("owner");
    address _alice = makeAddr("alice");
    address _bob = makeAddr("bob");
    WrappedToken _token;

    function setUp() public {
        _token = new WrappedToken("Token", "TKN", _owner);
    }

    function testTransferFromOnwer() public {
        assertEq(_token.balanceOf(_alice), 0);
        assertEq(_token.balanceOf(_owner), 0);

        vm.prank(_owner);
        _token.transfer(_alice, 100);

        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_owner), 0);
    }

    function testTransferToOwner() public {
        vm.prank(_owner);
        _token.transfer(_alice, 100);

        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_owner), 0);

        vm.prank(_alice);
        _token.transfer(_owner, 70);

        assertEq(_token.balanceOf(_alice), 30);
        assertEq(_token.balanceOf(_owner), 70);
    }

    function testTransferBetweenNonOwners() public {
        vm.prank(_owner);
        _token.transfer(_alice, 100);

        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_bob), 0);

        vm.prank(_alice);
        _token.transfer(_bob, 70);

        assertEq(_token.balanceOf(_alice), 30);
        assertEq(_token.balanceOf(_bob), 70);
    }

    function testTransferFromFromOwner() public {
        vm.prank(_owner);
        _token.transfer(_owner, 100);
        vm.prank(_owner);
        assertTrue(_token.approve(_alice, 70));

        assertEq(_token.allowance(_owner, _alice), 70);
        assertEq(_token.balanceOf(_alice), 0);
        assertEq(_token.balanceOf(_bob), 0);
        assertEq(_token.balanceOf(_owner), 100);

        vm.prank(_alice);
        _token.transferFrom(_owner, _bob, 60);

        assertEq(_token.balanceOf(_alice), 0);
        assertEq(_token.balanceOf(_bob), 60);
        assertEq(_token.balanceOf(_owner), 40);
        assertEq(_token.allowance(_owner, _alice), 10);
    }

    function testTransferFromToOwner() public {
        vm.prank(_owner);
        _token.transfer(_alice, 100);
        vm.prank(_alice);
        assertTrue(_token.approve(_owner, 70));

        assertEq(_token.allowance(_alice, _owner), 70);
        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_owner), 0);

        vm.prank(_owner);
        _token.transferFrom(_alice, _owner, 60);

        assertEq(_token.balanceOf(_alice), 40);
        assertEq(_token.balanceOf(_owner), 0);
        assertEq(_token.allowance(_alice, _owner), 10);
    }

    function testTransferFromToOwnerCalledByNonOwner() public {
        vm.prank(_owner);
        _token.transfer(_alice, 100);
        vm.prank(_alice);
        assertTrue(_token.approve(_bob, 70));

        assertEq(_token.allowance(_alice, _bob), 70);
        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_bob), 0);
        assertEq(_token.balanceOf(_owner), 0);

        vm.prank(_bob);
        _token.transferFrom(_alice, _owner, 60);

        assertEq(_token.balanceOf(_alice), 40);
        assertEq(_token.balanceOf(_owner), 60);
        assertEq(_token.balanceOf(_bob), 0);
        assertEq(_token.allowance(_alice, _bob), 10);
    }

    function testTransferFromBetweenNonOwners() public {
        vm.prank(_owner);
        _token.transfer(_alice, 100);
        vm.prank(_alice);
        assertTrue(_token.approve(_bob, 70));

        assertEq(_token.allowance(_alice, _bob), 70);
        assertEq(_token.balanceOf(_alice), 100);
        assertEq(_token.balanceOf(_bob), 0);
        assertEq(_token.balanceOf(_owner), 0);

        vm.prank(_bob);
        _token.transferFrom(_alice, _bob, 60);

        assertEq(_token.balanceOf(_alice), 40);
        assertEq(_token.balanceOf(_owner), 0);
        assertEq(_token.balanceOf(_bob), 60);
        assertEq(_token.allowance(_alice, _bob), 10);
    }

    function testSetMetadataSuccess() public {
        assertEq(_token.name(), "Token");
        assertEq(_token.symbol(), "TKN");
        assertEq(_token.decimals(), 18);

        vm.prank(_owner);
        _token.setMetaData(bytes32(bytes("New token")), bytes16(bytes("New symbol")), 42);
        assertEq(_token.name(), string(abi.encodePacked(bytes32(bytes("New token")))));
        assertEq(_token.symbol(), string(abi.encodePacked(bytes16(bytes("New symbol")))));
        assertEq(_token.decimals(), 42);

        vm.prank(_owner);
        _token.setMetaData(0x0, 0x0, 0);
        assertEq(_token.name(), string(abi.encodePacked(bytes32(bytes("New token")))));
        assertEq(_token.symbol(), string(abi.encodePacked(bytes16(bytes("New symbol")))));
        assertEq(_token.decimals(), 42);
    }

    function testSetMetadataInvalidCaller() public {
        vm.expectRevert("Unauthorised Access");
        _token.setMetaData(bytes32(bytes("New token")), bytes16(bytes("New symbol")), 42);
    }
}
