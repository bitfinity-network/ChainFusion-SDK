// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.17;

contract TestContractWithPrecompiles {
    // returns 0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45
    function do_keccak256() public pure returns (bytes32) {
        return (keccak256("abc"));
    }

    // returns ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    function do_sha256() public pure returns (bytes32) {
        return (sha256("abc"));
    }

    // returns 8eb208f7e05d987a9b044a8e98c6b087f15a0bfc000000000000000000000000
    function do_ripemd160() public pure returns (bytes20) {
        return (ripemd160("abc"));
    }

    // returns 7
    function do_addmod() public pure returns (uint256) {
        return (addmod(10, 6, 9));
    }

    // returns 1
    function do_mulmod() public pure returns (uint256) {
        return (mulmod(4, 4, 5));
    }

    // returns 0x228b2b113f2f1c6070991d2beca3db0f395158f4
    function do_ecrecover() public pure returns (address) {
        bytes32 _hash = 0x5cc4cee58087de1a2ea481fe9c65c92adc27cff464b7f00a486dc9bf6bb8efb3;
        bytes32 _r = 0x32573a0b258f251971a4ec35511c018a7e7bf75a5886534b48d12e47263048a2;
        bytes32 _s = 0xfe6e03543955255e235388b224704555fd036a954d3ee6dd030d9d1fea1830d7;
        uint8 _v = 0x1c;
        return (ecrecover(_hash, _v, _r, _s));
    }
}
