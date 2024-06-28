use std::borrow::Cow;
use std::fmt;
use std::mem::size_of;
use std::str::FromStr;

use bitcoin::hashes::sha256d::Hash;
use bitcoin::{Address, Amount, Network, OutPoint, TxOut, Txid};
use candid::{CandidType, Decode, Encode};
use ic_exports::ic_cdk::api::management_canister::bitcoin::{Outpoint, Utxo};
use ic_exports::ic_kit::ic;
use ic_stable_structures::stable_structures::DefaultMemoryImpl;
use ic_stable_structures::{BTreeMapStructure, Bound, StableBTreeMap, Storable, VirtualMemory};
use ord_rs::wallet::TxInputInfo;
use serde::Deserialize;

use crate::key::{ic_dp_to_derivation_path, IcBtcSigner};
use crate::memory::{LEDGER_MEMORY_ID, MEMORY_MANAGER, USED_UTXOS_REGISTRY_MEMORY_ID};

/// Data structure to keep track of utxos owned by the canister.
pub struct UtxoLedger {
    /// contains a list of utxos on the main canister account (which get there as a change from withdrawal transactions)
    utxo_storage: StableBTreeMap<UtxoKey, UtxoDetails, VirtualMemory<DefaultMemoryImpl>>,
    /// contains a list of utxos that are on user's deposit address.
    used_utxos_registry: StableBTreeMap<UtxoKey, UsedUtxoDetails, VirtualMemory<DefaultMemoryImpl>>,
}

impl Default for UtxoLedger {
    fn default() -> Self {
        Self {
            utxo_storage: StableBTreeMap::new(MEMORY_MANAGER.with(|mm| mm.get(LEDGER_MEMORY_ID))),
            used_utxos_registry: StableBTreeMap::new(
                MEMORY_MANAGER.with(|mm| mm.get(USED_UTXOS_REGISTRY_MEMORY_ID)),
            ),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, CandidType, Deserialize)]
pub struct UtxoKey {
    pub tx_id: [u8; 32],
    pub vout: u32,
}

impl fmt::Display for UtxoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", hex::encode(self.tx_id), self.vout)
    }
}

impl Storable for UtxoKey {
    fn to_bytes(&self) -> Cow<[u8]> {
        let bytes = Encode!(self).expect("cannot serialize utxo key");
        Cow::Owned(bytes)
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).expect("cannot deserialize utxo key")
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: 60,
        is_fixed_size: true,
    };
}

impl From<OutPoint> for UtxoKey {
    fn from(value: OutPoint) -> Self {
        Self {
            tx_id: *<Hash as AsRef<[u8; 32]>>::as_ref(&value.txid.to_raw_hash()),
            vout: value.vout,
        }
    }
}

impl From<&Outpoint> for UtxoKey {
    fn from(value: &Outpoint) -> Self {
        Self {
            tx_id: value.txid.clone().try_into().expect("invalid tx id"),
            vout: value.vout,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, CandidType, Deserialize)]
pub struct UtxoDetails {
    value: u64,
    script_buf: Vec<u8>,
    derivation_path: Vec<Vec<u8>>,
}

impl UtxoDetails {
    const MAX_SCRIPT_SIZE: u32 = 128;
    const DERIVATION_PATH_SIZE: u32 = IcBtcSigner::DERIVATION_PATH_SIZE;
}

impl Storable for UtxoDetails {
    fn to_bytes(&self) -> Cow<[u8]> {
        let bytes = Encode!(self).expect("failed to serialize utxo");
        Cow::Owned(bytes)
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).expect("failed to deserialize utxo")
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: size_of::<u64>() as u32 + Self::MAX_SCRIPT_SIZE + Self::DERIVATION_PATH_SIZE,
        is_fixed_size: false,
    };
}

#[derive(Debug, Clone, Eq, PartialEq, CandidType, Deserialize)]
pub struct UsedUtxoDetails {
    pub used_at: u64,
    owner_address: String,
}

impl UsedUtxoDetails {
    const MAX_BITCOIN_ADDRESS_SIZE: u32 = 96;

    /// Returns the owner address of the utxo.
    pub fn owner_address(&self, network: Network) -> Result<Address, Box<dyn std::error::Error>> {
        Address::from_str(self.owner_address.as_str())?
            .require_network(network)
            .map_err(|e| e.into())
    }
}

impl Storable for UsedUtxoDetails {
    fn to_bytes(&self) -> Cow<[u8]> {
        let bytes = Encode!(self).expect("failed to serialize utxo");
        Cow::Owned(bytes)
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).expect("failed to deserialize utxo")
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: size_of::<u64>() as u32 + Self::MAX_BITCOIN_ADDRESS_SIZE,
        is_fixed_size: false,
    };
}

impl UtxoLedger {
    /// Adds the utxo to the store.
    pub fn deposit(&mut self, utxos: &[Utxo], address: &Address, derivation_path: Vec<Vec<u8>>) {
        let script = address.script_pubkey();
        for utxo in utxos {
            self.utxo_storage.insert(
                (&utxo.outpoint).into(),
                UtxoDetails {
                    value: utxo.value,
                    script_buf: script.clone().into_bytes(),
                    derivation_path: derivation_path.clone(),
                },
            );

            log::debug!(
                "Added utxo {}:{} with value {} to the ledger",
                hex::encode(&utxo.outpoint.txid),
                utxo.outpoint.vout,
                utxo.value
            );
        }
    }

    /// Lists all unspent utxos in the store.
    pub fn load_unspent_utxos(&self) -> (Vec<UtxoKey>, Vec<TxInputInfo>) {
        self.utxo_storage
            .iter()
            .filter(|(key, _)| !self.used_utxos_registry.contains_key(key))
            .map(|(key, details)| {
                (
                    key,
                    TxInputInfo {
                        outpoint: OutPoint {
                            txid: Txid::from_raw_hash(*Hash::from_bytes_ref(&key.tx_id)),
                            vout: key.vout,
                        },
                        tx_out: TxOut {
                            value: Amount::from_sat(details.value),
                            script_pubkey: details.script_buf.into(),
                        },
                        derivation_path: ic_dp_to_derivation_path(&details.derivation_path),
                    },
                )
            })
            .unzip()
    }

    /// Marks the utxo as used.
    pub fn mark_as_used(&mut self, key: UtxoKey, address: Address) {
        self.used_utxos_registry.insert(
            key,
            UsedUtxoDetails {
                used_at: ic::time(),
                owner_address: address.to_string(),
            },
        );

        log::trace!("Utxo {key} is marked as used.");
    }

    /// Lists all used utxos in the store.
    pub fn load_used_utxos(&self) -> Vec<(UtxoKey, UsedUtxoDetails)> {
        self.used_utxos_registry.iter().collect()
    }

    /// Removes the spent utxo from the store.
    ///
    /// It gets removed from both the utxo storage and the used utxos registry.
    pub fn remove_spent_utxo(&mut self, key: &UtxoKey) {
        self.utxo_storage.remove(key);
        self.used_utxos_registry.remove(key);
    }

    /// Removes the unspent utxo from the store.
    /// It gets removed only from the `used_utxos_registry`
    pub fn remove_unspent_utxo(&mut self, key: &UtxoKey) {
        self.used_utxos_registry.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Network, PublicKey};
    use did::H160;
    use ic_exports::ic_kit::MockContext;

    use super::*;
    use crate::canister::get_state;
    use crate::key::get_derivation_path_ic;

    #[test]
    fn key_serialization() {
        let key = UtxoKey {
            tx_id: [123; 32],
            vout: 544331,
        };

        let serialized = key.to_bytes();
        let Bound::Bounded { max_size, .. } = UtxoKey::BOUND else {
            panic!("Key is unbounded");
        };

        assert_eq!(serialized.len() as u32, max_size);
        let deserialized = UtxoKey::from_bytes(serialized);
        assert_eq!(deserialized, key);
    }

    #[test]
    fn value_serialization() {
        let address = Address::p2wpkh(
            &PublicKey::from_str(
                "038f47dcd43ba6d97fc9ed2e3bba09b175a45fac55f0683e8cf771e8ced4572354",
            )
            .unwrap(),
            Network::Signet,
        )
        .unwrap();
        let derivation_path = get_derivation_path_ic(
            &H160::from_hex_str("0x0dc9f6938e9b47fd8553df50bcbdb62d67239007").unwrap(),
        );
        let value = UtxoDetails {
            value: 100500,
            script_buf: address.script_pubkey().to_bytes(),
            derivation_path,
        };

        let serialized = value.to_bytes();
        let Bound::Bounded { max_size, .. } = UtxoDetails::BOUND else {
            panic!("Key is unbounded");
        };

        assert!((serialized.len() as u32) < max_size);
        let deserialized = UtxoDetails::from_bytes(serialized);
        assert_eq!(deserialized, value);
    }

    #[test]
    fn test_should_serialize_used_utxo() {
        let address = Address::p2wpkh(
            &PublicKey::from_str(
                "038f47dcd43ba6d97fc9ed2e3bba09b175a45fac55f0683e8cf771e8ced4572354",
            )
            .unwrap(),
            Network::Signet,
        )
        .unwrap();
        let value = UsedUtxoDetails {
            used_at: 100500,
            owner_address: address.to_string(),
        };

        let serialized = value.to_bytes();
        let Bound::Bounded { max_size, .. } = UsedUtxoDetails::BOUND else {
            panic!("Key is unbounded");
        };

        assert!((serialized.len() as u32) < max_size);
        let deserialized = UsedUtxoDetails::from_bytes(serialized);
        assert_eq!(deserialized, value);
    }

    #[test]
    fn test_should_deposit_utxo() {
        MockContext::new().inject();
        let address = Address::from_str("bc1quyjp8qxkdc22cej962xaydd5arm7trwtcnkzks")
            .unwrap()
            .assume_checked();

        let utxo = Utxo {
            outpoint: Outpoint {
                txid: vec![0xde; 32],
                vout: 1,
            },
            value: 0,
            height: 0,
        };

        let state = get_state();
        state
            .borrow_mut()
            .ledger_mut()
            .deposit(&[utxo], &address, vec![]);

        // list unspent
        let (keys, _) = state.borrow().ledger().load_unspent_utxos();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].tx_id, [0xde; 32]);
        assert_eq!(keys[0].vout, 1);
    }

    #[test]
    fn test_should_mark_used_utxo() {
        MockContext::new().inject();
        let address = Address::from_str("bc1quyjp8qxkdc22cej962xaydd5arm7trwtcnkzks")
            .unwrap()
            .assume_checked();

        let utxo = Utxo {
            outpoint: Outpoint {
                txid: vec![0xde; 32],
                vout: 1,
            },
            value: 0,
            height: 0,
        };

        let state = get_state();
        state
            .borrow_mut()
            .ledger_mut()
            .deposit(&[utxo], &address, vec![]);

        let (keys, _) = state.borrow().ledger().load_unspent_utxos();

        state
            .borrow_mut()
            .ledger_mut()
            .mark_as_used(keys[0], address.clone());

        let used_utxos = state.borrow().ledger().load_used_utxos();
        assert_eq!(used_utxos.len(), 1);
        assert_eq!(used_utxos[0].0, keys[0]);
        assert_eq!(used_utxos[0].1.owner_address, address.to_string());
    }

    #[test]
    fn test_should_not_list_unspent_utxo_if_used() {
        MockContext::new().inject();
        let address = Address::from_str("bc1quyjp8qxkdc22cej962xaydd5arm7trwtcnkzks")
            .unwrap()
            .assume_checked();

        let utxos = vec![
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xaa; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xab; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
        ];

        let state = get_state();
        state
            .borrow_mut()
            .ledger_mut()
            .deposit(&utxos, &address, vec![]);

        // mark first as spent
        state
            .borrow_mut()
            .ledger_mut()
            .mark_as_used(UtxoKey::from(&utxos[0].outpoint), address.clone());

        // load unspent
        let (keys, _) = state.borrow().ledger().load_unspent_utxos();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].tx_id.to_vec(), utxos[1].outpoint.txid);
        assert_eq!(keys[0].vout, utxos[1].outpoint.vout);

        // load used
        let used_utxos = state.borrow().ledger().load_used_utxos();
        assert_eq!(used_utxos.len(), 1);
        assert_eq!(used_utxos[0].0.tx_id.to_vec(), utxos[0].outpoint.txid);
        assert_eq!(used_utxos[0].0.vout, utxos[0].outpoint.vout);
    }

    #[test]
    fn test_should_remove_spent_utxo() {
        MockContext::new().inject();
        let address = Address::from_str("bc1quyjp8qxkdc22cej962xaydd5arm7trwtcnkzks")
            .unwrap()
            .assume_checked();

        let utxos = vec![
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xaa; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xab; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
        ];

        let state = get_state();
        state
            .borrow_mut()
            .ledger_mut()
            .deposit(&utxos, &address, vec![]);

        // mark first as spent
        state
            .borrow_mut()
            .ledger_mut()
            .mark_as_used(UtxoKey::from(&utxos[0].outpoint), address.clone());

        // remove spent
        state
            .borrow_mut()
            .ledger_mut()
            .remove_spent_utxo(&UtxoKey::from(&utxos[0].outpoint));

        // check
        let (keys, _) = state.borrow().ledger().load_unspent_utxos();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].tx_id.to_vec(), utxos[1].outpoint.txid);
        assert_eq!(keys[0].vout, utxos[1].outpoint.vout);

        let used_utxos = state.borrow().ledger().load_used_utxos();
        assert_eq!(used_utxos.len(), 0);
    }

    #[test]
    fn test_should_remove_unspent_utxo() {
        MockContext::new().inject();
        let address = Address::from_str("bc1quyjp8qxkdc22cej962xaydd5arm7trwtcnkzks")
            .unwrap()
            .assume_checked();

        let utxos = vec![
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xaa; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0xab; 32],
                    vout: 1,
                },
                value: 0,
                height: 0,
            },
        ];

        let state = get_state();
        state
            .borrow_mut()
            .ledger_mut()
            .deposit(&utxos, &address, vec![]);

        // mark first as spent
        state
            .borrow_mut()
            .ledger_mut()
            .mark_as_used(UtxoKey::from(&utxos[0].outpoint), address.clone());

        // remove spent
        state
            .borrow_mut()
            .ledger_mut()
            .remove_unspent_utxo(&UtxoKey::from(&utxos[0].outpoint));

        // check
        let (keys, _) = state.borrow().ledger().load_unspent_utxos();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].tx_id.to_vec(), utxos[0].outpoint.txid);
        assert_eq!(keys[0].vout, utxos[0].outpoint.vout);
        assert_eq!(keys[1].tx_id.to_vec(), utxos[1].outpoint.txid);
        assert_eq!(keys[1].vout, utxos[1].outpoint.vout);

        let used_utxos = state.borrow().ledger().load_used_utxos();
        assert_eq!(used_utxos.len(), 0);
    }
}
