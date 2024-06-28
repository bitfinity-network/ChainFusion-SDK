// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.17;

contract TestContractWithEvents {
    event valueEvent(uint256 indexed oldNumber, uint256 indexed newNumber, uint256 valueIndex, address sender);

    uint256 a;
    uint256 b;

    constructor() {
        a = 11;
        emit valueEvent(0, a, 1, msg.sender);
        b = 22;
        emit valueEvent(0, b, 2, msg.sender);
    }

    function getA() public view returns (uint256) {
        return a;
    }

    function setA(uint256 val) public returns (uint256) {
        emit valueEvent(a, val, 1, msg.sender);
        a = val;
        return a;
    }
}
