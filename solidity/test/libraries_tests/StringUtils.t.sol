// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

import "forge-std/Test.sol";
import "forge-std/console.sol";
import "src/libraries/StringUtils.sol";

contract StringUtilsTest is Test {
    function bytes32ToString(bytes32 _bytes32) public pure returns (string memory) {
        uint8 i = 0;
        while (i < 32 && _bytes32[i] != 0) {
            i++;
        }

        bytes memory bytesArray = new bytes(i);
        for (i = 0; i < 32 && _bytes32[i] != 0; i++) {
            bytesArray[i] = _bytes32[i];
        }

        return string(bytesArray);
    }

    function testTruncateUTF8() public pure {
        {
            bytes32 result = StringUtils.truncateUTF8("");
            assertEq(bytes32ToString(result), "");
        }

        {
            bytes32 result = StringUtils.truncateUTF8("1");
            assertEq(bytes32ToString(result), "1");
        }

        {
            bytes32 result = StringUtils.truncateUTF8("123");
            assertEq(bytes32ToString(result), "123");
        }

        {
            bytes32 result = StringUtils.truncateUTF8("12345678901234567890123456789012");
            assertEq(bytes32ToString(result), "12345678901234567890123456789012");
        }
        {
            bytes32 result = StringUtils.truncateUTF8(unicode"1234567890123456789012345678901ї");
            assertEq(bytes32ToString(result), unicode"1234567890123456789012345678901");
        }

        {
            bytes32 result = StringUtils.truncateUTF8(unicode"123456789012345678901234567890ї");
            assertEq(bytes32ToString(result), unicode"123456789012345678901234567890");
        }

        {
            bytes32 result = StringUtils.truncateUTF8(unicode"123456789012345678ї");
            assertEq(bytes32ToString(result), unicode"123456789012345678ї");
        }

        {
            bytes32 result = StringUtils.truncateUTF8(unicode"12345678901234567890її1");
            assertEq(bytes32ToString(result), unicode"12345678901234567890її1");
        }

        {
            bytes32 result = StringUtils.truncateUTF8(unicode"ї");
            assertEq(bytes32ToString(result), unicode"ї");
        }
    }
}
