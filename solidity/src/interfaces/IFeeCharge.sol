// SPDX-License-Identifier: MIT
pragma solidity >=0.5.0;

// Allows to depsit/withdraw native tokens and use the deposit to charge fee.
interface IFeeCharge {
    // Deposit `msg.value` amount of native token to user's address.
    // The deposit could be used to pay fees by the approvedSenderIDs.
    // Returns user's balance after the operation.
    function nativeTokenDeposit(bytes32[] calldata approvedSenderIDs) external payable returns (uint256 balance);

    // Withdraw the amount of native token to user's address.
    // Returns user's balance after the operation.
    function nativeTokenWithdraw(uint256 amount) external payable returns (uint256 balance);

    // Returns user's native token deposit balance.
    function nativeTokenBalance(address user) external view returns (uint256 balance);

    // Remove approved SpenderIDs
    function removeApprovedSenderIDs(bytes32[] calldata approvedSenderIDs) external;

    // Take the given amount of fee from the user.
    // Require the user to have enough native token balance and approval for senderID.
    function chargeFee(address from, address payable to, bytes32 senderID, uint256 amount) external;
}
