// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

library StringUtils {
    // Function to truncate UTF8 strings
    function truncateUTF8(string memory input) internal pure returns (bytes32 result) {
        // If the last byte starts with 0xxxxx, return the data as is
        bytes memory source = bytes(input);
        if (source.length < 32 || (source[31] & 0x80) == 0) {
            assembly {
                result := mload(add(source, 32))
            }
            return result;
        }

        if (source.length == 0) {
            return 0x0;
        }

        // Go backwards from the last byte until a byte that doesn't start with 10xxxxxx is found
        for (uint8 i = 31; i >= 0; i--) {
            if ((source[i] & 0xC0) != 0x80) {
                for (uint8 j = i; j < 32; j += 1) {
                    source[j] = 0;
                }

                assembly {
                    result := mload(add(source, 32))
                }

                break;
            }

            if (i == 0) {
                return 0x0;
            }
        }
    }
}
