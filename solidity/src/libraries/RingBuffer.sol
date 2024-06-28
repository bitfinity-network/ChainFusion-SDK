// SPDX-License-Identifier: MIT
pragma solidity ^0.8.7;

library RingBuffer {
    struct RingBufferUint32 {
        uint8 begin;
        uint8 end;
        mapping(uint8 => uint32) values;
    }

    // Append the value to the ring buffer.
    function push(RingBufferUint32 storage buffer, uint32 value) internal {
        buffer.values[buffer.end] = value;

        unchecked {
            buffer.end++;
        }

        if (buffer.begin == buffer.end) {
            unchecked {
                buffer.begin++;
            }
        }
    }

    // Function to get the size of the buffer.
    function size(RingBufferUint32 storage buffer) internal view returns (uint8 sizeOf) {
        if (buffer.begin <= buffer.end) {
            sizeOf = buffer.end - buffer.begin;
        } else {
            sizeOf = 255;
        }
    }

    // Returns all values in order of pushing.
    function getAll(RingBufferUint32 storage buffer) internal view returns (uint32[] memory values) {
        uint8 _size = size(buffer);
        values = new uint32[](_size);
        for (uint8 i = 0; i < _size; i++) {
            uint8 offset;
            unchecked {
                offset = buffer.begin + i;
            }
            uint32 value = buffer.values[offset];
            values[i] = value;
        }
    }
}
