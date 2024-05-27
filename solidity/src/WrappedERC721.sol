// SPDX-License-Identifier: UNLICENSED

pragma solidity ^0.8.7;

import {IERC721} from "openzeppelin-contracts/token/ERC721/IERC721.sol";
import {ERC721} from "openzeppelin-contracts/token/ERC721/ERC721.sol";
import {ERC721URIStorage} from "openzeppelin-contracts/token/ERC721/extensions/ERC721URIStorage.sol";
import {ERC721Enumerable} from "openzeppelin-contracts/token/ERC721/extensions/ERC721Enumerable.sol";
import {Ownable} from "openzeppelin-contracts/access/Ownable.sol";
import "src/WrappedToken.sol";

// Custom token contract based on ERC 721,
contract WrappedERC721 is ERC721URIStorage, ERC721Enumerable, Ownable {
    string private _name;
    string private _symbol;

    uint256 private _tokenIdCounter;

    // Initializes contract with the given name and symbl
    constructor(
        string memory name_,
        string memory symbol_,
        address _owner
    ) ERC721(name_, symbol_) Ownable(_owner) {
        _tokenIdCounter = 0;
        _name = name_;
        _symbol = symbol_;
    }

    /// @notice mint a new NFT
    /// @param _receiver address to give the nft to
    /// @param _uri token uri
    /// @return _newItemId the id of the minted token
    function safeMint(
        address _receiver,
        string memory _uri
    ) public onlyOwner returns (uint256 _newItemId) {
        _tokenIdCounter += 1;
        uint256 tokenId = _tokenIdCounter;
        _safeMint(_receiver, tokenId);
        _setTokenURI(tokenId, _uri);

        return tokenId;
    }

    /// @notice This function allows the owner to burn a token.
    /// @param tokenId The token to burn.
    function burn(uint256 tokenId) public onlyOwner {
        _burn(tokenId);
        // delete data
        _setTokenURI(tokenId, "");
    }

    /// This function allows the owner to change other wallet allowance.
    function approveByOwner(
        address from,
        address spender,
        uint256 tokenId
    ) public virtual onlyOwner {
        _approve(spender, tokenId, from);
    }

    // Updates token name, symbol and decimals if needed.
    function setMetaData(bytes32 name_, bytes16 symbol_) public onlyOwner {
        if (symbol_ != 0x0) {
            if (bytes16(bytes(_symbol)) != symbol_) {
                _symbol = string(abi.encodePacked(symbol_));
            }
        }
        if (name_ != 0x0) {
            if (bytes32(bytes(_name)) != name_) {
                _name = string(abi.encodePacked(name_));
            }
        }
    }

    /// @notice This function allows to get the symbol of the token.
    /// @return The symbol of the token.
    function symbol() public view override returns (string memory) {
        return _symbol;
    }

    /// @notice This function allows to get the name of the token.
    /// @return The name of the token.
    function name() public view override returns (string memory) {
        return _name;
    }

    function supportsInterface(
        bytes4 interfaceId
    ) public view override(ERC721URIStorage, ERC721Enumerable) returns (bool) {
        return super.supportsInterface(interfaceId);
    }

    function _increaseBalance(
        address account,
        uint128 value
    ) internal override(ERC721, ERC721Enumerable) {
        super._increaseBalance(account, value);
    }

    function _update(
        address to,
        uint256 tokenId,
        address auth
    ) internal override(ERC721, ERC721Enumerable) returns (address) {
        return super._update(to, tokenId, auth);
    }

    function tokenURI(
        uint256 tokenId
    ) public view override(ERC721, ERC721URIStorage) returns (string memory) {
        return super.tokenURI(tokenId);
    }
}
