// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.17;

contract TestRevertMessage {
    function revertWithMessage() public pure {
        revert("This is a revert message");
    }

    function requireRevertWithMessage() public pure {
        require(false, "This is a require revert message");
    }
}
