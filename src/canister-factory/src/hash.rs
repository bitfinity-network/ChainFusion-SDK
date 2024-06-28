use sha3::{Digest, Keccak256};

pub fn hash_wasm_module(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn hash_wasm_module_hex(data: &[u8]) -> String {
    let hash = hash_wasm_module(data);
    hex::encode(hash)
}
