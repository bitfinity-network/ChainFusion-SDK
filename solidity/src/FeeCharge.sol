// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "src/interfaces/IFeeCharge.sol";
import { RingBuffer } from "src/libraries/RingBuffer.sol";

contract FeeCharge is IFeeCharge {
    // Mapping from user address to amount of native tokens on his deposit.
    mapping(address => uint256) private _userBalance;

    // Mapping from user address to list of senderIDs, which are able to spend native deposit.
    mapping(address => mapping(bytes32 => bool)) private _approvedIDs;

    mapping(address => bool) private _canChargeFee;

    constructor(address[] memory canChargeFee) {
        uint256 length = canChargeFee.length;
        for (uint256 i = 0; i < length; i++) {
            address approved = canChargeFee[i];
            _canChargeFee[approved] = true;
        }
    }

    // Deposit `msg.value` amount of native token to user's address.
    // The deposit could be used to pay fees by the approvedSenderIDs.
    // Returns user's balance after the operation.
    function nativeTokenDeposit(bytes32[] calldata approvedSenderIDs) external payable returns (uint256 balance) {
        address to = msg.sender;

        // Add approved SpenderIDs
        for (uint256 i = 0; i < approvedSenderIDs.length; i++) {
            _approvedIDs[to][approvedSenderIDs[i]] = true;
        }

        balance = _userBalance[to];
        balance += msg.value;
        _userBalance[to] = balance;
    }

    // Withdraw the amount of native token to user's address.
    // Returns user's balance after the operation.
    function nativeTokenWithdraw(uint256 amount) external payable returns (uint256 balance) {
        require(amount > 0, "failed to withdraw zero amount");
        address to = msg.sender;

        balance = _userBalance[to];
        require(balance >= amount, "insufficient balance to withdraw");
        balance -= amount;
        _userBalance[to] = balance;
        payable(to).transfer(amount);
    }

    // Returns user's native token deposit balance.
    function nativeTokenBalance(address user) external view returns (uint256 balance) {
        if (user == address(0)) {
            user = msg.sender;
        }
        balance = _userBalance[user];
    }

    // Remove approved SpenderIDs
    function removeApprovedSenderIDs(bytes32[] calldata approvedSenderIDs) external {
        for (uint256 i = 0; i < approvedSenderIDs.length; i++) {
            delete _approvedIDs[msg.sender][approvedSenderIDs[i]];
        }
    }

    // Take the given amount of fee from the user.
    // Require the user to have enough native token balance and approval for senderID.
    function chargeFee(address from, address payable to, bytes32 senderID, uint256 amount) external {
        require(_canChargeFee[msg.sender], "fee charger is not present in allow list");
        uint256 balance = _userBalance[from];
        require(balance >= amount, "insufficient balance to pay fee");
        require(_approvedIDs[from][senderID], "senderID is not approved");

        uint256 newBalance = balance - amount;
        _userBalance[from] = newBalance;
        to.transfer(amount);
    }
}
