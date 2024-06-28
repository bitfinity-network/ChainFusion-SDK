// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "forge-std/console.sol";
import { RingBuffer } from "src/libraries/RingBuffer.sol";

contract RingBufferTests is Test {
    using RingBuffer for RingBuffer.RingBufferUint32;

    RingBuffer.RingBufferUint32 _buffer;

    function testIncrementRingBuffer() public {
        assertEq(_buffer.size(), 0);

        for (uint32 i = 1; i < 256; i += 1) {
            _buffer.push(i);
            assertEq(_buffer.size(), i);
            assertEq(_buffer.getAll().length, i);
        }
        assertEq(_buffer.getAll()[0], 1);

        _buffer.push(300);
        assertEq(_buffer.size(), 255);
        assertEq(_buffer.getAll()[0], 2);
        assertEq(_buffer.getAll()[254], 300);

        _buffer.push(301);
        assertEq(_buffer.size(), 255);
        assertEq(_buffer.getAll()[0], 3);
        assertEq(_buffer.getAll()[254], 301);

        _buffer.push(302);
        assertEq(_buffer.size(), 255);
        assertEq(_buffer.getAll()[0], 4);
        assertEq(_buffer.getAll()[254], 302);
    }
}
