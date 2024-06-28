// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.17;

import "forge-std/console.sol";

contract TestLog {
    event logString(string message);

    // returns 0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45
    function do_log() public returns (bytes32) {
        // a log emitted from Solidity which is printed in the application logs by the
        // evm_core REVM LogInspector
        emit logString("-------------------- An emit message from Solidity --------------------");

        // This is from forge-std/console.sol
        // Currently this prints nothing :\
        console.logString("-------------------- A console.log from Solidity --------------------");

        return (keccak256("abc"));
    }
}
