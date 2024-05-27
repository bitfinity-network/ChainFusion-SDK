use ethers_core::abi::{Constructor, Function, Param, ParamType, StateMutability};
use once_cell::sync::Lazy;

pub static CONSTRUCTOR: Lazy<Constructor> = Lazy::new(|| Constructor {
    inputs: vec![
        Param {
            name: "name".into(),
            kind: ParamType::String,
            internal_type: None,
        },
        Param {
            name: "symbol".into(),
            kind: ParamType::String,
            internal_type: None,
        },
        Param {
            name: "owner".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
    ],
});

#[allow(deprecated)] // need to initialize `constant` field
pub static TRANSFER_FROM: Lazy<Function> = Lazy::new(|| Function {
    name: "transferFrom".into(),
    inputs: vec![
        Param {
            name: "from".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
        Param {
            name: "to".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
        Param {
            name: "tokenId".into(),
            kind: ParamType::Uint(256),
            internal_type: None,
        },
    ],
    outputs: vec![],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static APPROVE_BY_OWNER: Lazy<Function> = Lazy::new(|| Function {
    name: "approveByOwner".into(),
    inputs: vec![
        Param {
            name: "from".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
        Param {
            name: "spender".into(),
            kind: ParamType::Address,
            internal_type: None,
        },
        Param {
            name: "tokenId".into(),
            kind: ParamType::Uint(256),
            internal_type: None,
        },
    ],
    outputs: vec![],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static BALANCE_OF: Lazy<Function> = Lazy::new(|| Function {
    name: "balanceOf".into(),
    inputs: vec![Param {
        name: "owner".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
    outputs: vec![Param {
        name: "".into(),
        kind: ParamType::Uint(256),
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::NonPayable,
});

#[allow(deprecated)] // need to initialize `constant` field
pub static GET_APPROVED: Lazy<Function> = Lazy::new(|| Function {
    name: "getApproved".into(),
    inputs: vec![Param {
        name: "tokenId".into(),
        kind: ParamType::Uint(256),
        internal_type: None,
    }],
    outputs: vec![Param {
        name: "operator".into(),
        kind: ParamType::Address,
        internal_type: None,
    }],
    constant: None,
    state_mutability: StateMutability::View,
});
