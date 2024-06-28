// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.17;

contract TestEnvVariable {
    function getTime() public view returns (uint256) {
        return block.timestamp;
    }

    function getBlockNumber() public view returns (uint256) {
        return block.number;
    }
}
