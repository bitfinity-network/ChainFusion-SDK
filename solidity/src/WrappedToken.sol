// SPDX-License-Identifier: MIT

pragma solidity ^0.8.7;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

// Custom token contract based on ERC 20,
contract WrappedToken is ERC20 {
    address public immutable owner;
    string private _name;
    string private _symbol;
    uint8 private _decimals;

    // Initializes contract with the given name and symbl
    constructor(string memory name_, string memory symbol_, address _owner) ERC20(name_, symbol_) {
        owner = _owner;
        _name = name_;
        _symbol = symbol_;
        _decimals = super.decimals();
    }

    // Perform IERC20 transfer.
    // If `msg.sender` is `owner` then mint happens.
    function transfer(address to, uint256 value) public virtual override returns (bool) {
        if (msg.sender == owner) {
            _mint(owner, value);
        }
        bool success = super.transfer(to, value);

        // revert if fail
        if (msg.sender == owner && !success) {
            _burn(owner, value);
        }
        return success;
    }

    /// This function allows the owner to change other wallet allowance.
    function approveByOwner(address from, address spender, uint256 value) public virtual returns (bool) {
        if (_msgSender() != owner) {
            return false;
        }

        _approve(from, spender, value);
        return true;
    }

    // Perform IERC20 transfer from `sender` address.
    // If called by `owner` and `recipient` is `owner` then burn happens.
    function transferFrom(address sender, address recipient, uint256 amount) public virtual override returns (bool) {
        bool success = super.transferFrom(sender, recipient, amount);
        if (msg.sender == owner && recipient == owner && success) {
            _burn(owner, amount);
        }
        return success;
    }

    // Updates token name, symbol and decimals if needed.
    function setMetaData(bytes32 name_, bytes16 symbol_, uint8 decimals_) public {
        require(msg.sender == owner, "Unauthorised Access");
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
        if (decimals_ != 0 && _decimals != decimals_) {
            _decimals = decimals_;
        }
    }

    // Returns the name of the token.
    function name() public view virtual override returns (string memory) {
        return _name;
    }

    //Returns the symbol of the token, usually a shorter version of the
    function symbol() public view virtual override returns (string memory) {
        return _symbol;
    }

    // Returns the number of decimals used to get its user representation.
    // For example, if `decimals` equals `2`, a balance of `505` tokens should
    // be displayed to a user as `5.05` (`505 / 10 ** 2`).
    //
    // Tokens usually opt for a value of 18, imitating the relationship between
    // Ether and Wei. This is the value {ERC20} uses, unless this function is
    // overridden;
    //
    // NOTE: This information is only used for _display_ purposes: it in
    // no way affects any of the arithmetic of the contract, including
    // {IERC20-balanceOf} and {IERC20-transfer}.
    function decimals() public view virtual override returns (uint8) {
        return _decimals;
    }
}
