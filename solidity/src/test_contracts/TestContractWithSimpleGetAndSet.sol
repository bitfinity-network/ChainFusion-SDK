// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.17;

contract TestContractWithSimpleGetAndSet {
    uint256 a;
    uint256 b;

    constructor() {
        a = 11;
        b = 22;
    }

    function getA() public view returns (uint256) {
        return a;
    }

    function setA(uint256 val) public returns (uint256) {
        a = val;
        return a;
    }
}
