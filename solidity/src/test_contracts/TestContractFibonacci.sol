// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.17;

contract TestContractFibonacci {
    uint256 last;

    constructor() {
        last = 0;
    }

    function fib(uint256 n) public returns (uint256 b) {
        if (n == 0) {
            return 0;
        }
        uint256 a = 1;
        b = 1;
        for (uint256 i = 2; i < n; i++) {
            uint256 c = a + b;
            a = b;
            b = c;
        }

        last = b;

        return b;
    }
}
