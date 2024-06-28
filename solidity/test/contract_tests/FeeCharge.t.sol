// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "src/FeeCharge.sol";

contract ChargeFeeTest is Test {
    address _alice = makeAddr("alice");
    bytes32 _aliceSender1 = bytes32(uint256(1));
    bytes32 _aliceSender2 = bytes32(uint256(2));
    uint256 _aliceInitBalance = 10 ** 18;
    uint256 _aliceInitDeposit = 10 ** 17;

    address _bob = makeAddr("bob");
    bytes32 _bobSender1 = bytes32(uint256(3));
    uint256 _bobInitDeposit = 0;

    address _charger = makeAddr("charger");
    address _recepient = makeAddr("recepient");

    FeeCharge _feeCharge;

    function setUp() public {
        address[] memory chargers = new address[](1);
        chargers[0] = _charger;
        _feeCharge = new FeeCharge(chargers);

        bytes32[] memory aliceSenderIDs = new bytes32[](2);
        aliceSenderIDs[0] = _aliceSender1;
        aliceSenderIDs[1] = _aliceSender2;
        vm.deal(_alice, _aliceInitBalance);
        vm.prank(_alice);
        _feeCharge.nativeTokenDeposit{ value: _aliceInitDeposit }(aliceSenderIDs);
    }

    function testDeposit() public view {
        assertEq(_alice.balance, _aliceInitBalance - _aliceInitDeposit);
        uint256 aliceBalance = _feeCharge.nativeTokenBalance(_alice);
        assertEq(aliceBalance, _aliceInitDeposit);

        uint256 bobBalance = _feeCharge.nativeTokenBalance(_bob);
        assertEq(bobBalance, _bobInitDeposit);
    }

    function testWithdraw() public {
        uint256 amount = 1000;
        vm.prank(_alice);
        uint256 newBalance = _feeCharge.nativeTokenWithdraw(amount);
        assertEq(newBalance, _aliceInitDeposit - amount, "deposit balance should decrese");
        assertEq(_alice.balance, _aliceInitBalance - _aliceInitDeposit + amount, "alice balance should decrease");

        vm.prank(_bob);
        vm.expectRevert();
        _feeCharge.nativeTokenWithdraw(amount);
    }

    function testFeeCharge() public {
        uint256 fee = 1000;

        vm.prank(_charger);
        _feeCharge.chargeFee(_alice, payable(_recepient), _aliceSender1, fee);
        uint256 newBalance = _feeCharge.nativeTokenBalance(_alice);
        assertEq(newBalance, _aliceInitDeposit - fee, "deposit balance should decrese");
        assertEq(_alice.balance, _aliceInitBalance - _aliceInitDeposit, "alice balance should not change");
        assertEq(_recepient.balance, fee, "recepient balance should increase");

        vm.prank(_charger);
        _feeCharge.chargeFee(_alice, payable(_recepient), _aliceSender2, fee);
        uint256 newBalance2 = _feeCharge.nativeTokenBalance(_alice);
        assertEq(newBalance2, _aliceInitDeposit - fee * 2, "deposit balance should decrese");
        assertEq(_alice.balance, _aliceInitBalance - _aliceInitDeposit, "alice balance should not change");
        assertEq(_recepient.balance, fee * 2, "recepient balance should increase");

        vm.prank(_alice);
        vm.expectRevert();
        _feeCharge.chargeFee(_alice, payable(_recepient), _aliceSender1, fee);

        vm.prank(_charger);
        vm.expectRevert();
        _feeCharge.chargeFee(_alice, payable(_recepient), _bobSender1, fee);
    }
}
