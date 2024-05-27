use bitcoin::Amount;
use ic_exports::ic_cdk::api::management_canister::bitcoin::Utxo;
use serde::{Deserialize, Serialize};

use crate::interface::{InscribeError, InscribeResult, InscriptionFees};

#[derive(Default)]
pub struct UtxoStore {
    inner: Vec<UtxoManager>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct UtxoManager {
    utxo: Utxo,
    purpose: UtxoType,
    value: Amount,
}

/// Classification of a UTXO based on its purpose.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq)]
enum UtxoType {
    /// Denotes a UTXO earmarked for inscription.
    Inscription,
    /// Denotes a UTXO used to pay for transaction fees
    #[default]
    Fee,
    /// Denotes a UTXO left after fees have been deducted
    Leftover,
}

impl UtxoStore {
    /// Prepares UTXOs for a commit transaction and classifies leftovers.
    ///
    /// # Arguments
    ///
    /// * `fees` - The total fees (including the `POSTAGE`) required for the transaction.
    /// * `fetched_utxos` - The list of UTXOs available for the transaction.
    ///
    /// # Returns
    ///
    /// A result containing either a list of UTXOs selected for the inscription
    /// or an error if there are insufficient funds.
    pub(crate) fn process_utxos(
        &mut self,
        fees: InscriptionFees,
        fetched_utxos: Vec<Utxo>,
    ) -> InscribeResult<Vec<Utxo>> {
        // Sort the UTXOs by value.
        let validated_utxos = self.sort_utxos(fetched_utxos);
        log::info!("Number of UTXOs: {}", validated_utxos.len());

        let InscriptionFees {
            postage,
            commit_fee,
            reveal_fee,
            ..
        } = fees;

        let required_utxo_sum = postage + commit_fee + reveal_fee;
        log::info!("Required UTXO total: {}", required_utxo_sum);

        let actual_utxo_sum = validated_utxos.iter().map(|utxo| utxo.value).sum::<u64>();
        log::info!("Actual UTXO total: {}", actual_utxo_sum);

        if actual_utxo_sum < required_utxo_sum {
            return Err(InscribeError::InsufficientFundsForFees(format!(
                "Actual UTXO sum: {}. Required sum: {}",
                actual_utxo_sum, required_utxo_sum
            )));
        }

        let mut selected_utxos = Vec::new();
        let mut selected_utxos_sum = 0u64;

        for utxo in validated_utxos.into_iter() {
            if selected_utxos_sum < required_utxo_sum {
                selected_utxos_sum += utxo.value;
                selected_utxos.push(utxo);
            } else {
                // Once enough UTXOs are selected, classify any additional as `Leftover`
                self.classify_utxo(&utxo, UtxoType::Leftover, Amount::from_sat(utxo.value));
            }
        }

        if selected_utxos_sum < required_utxo_sum {
            return Err(InscribeError::InsufficientFundsForInscriptions(format!(
                "Available: {}, Required: >= {}",
                selected_utxos_sum, required_utxo_sum
            )));
        }

        Ok(selected_utxos)
    }

    /// Resets the UTXO vault of.
    ///
    /// This method clears all UTXO tracking within the state, effectively removing
    /// all UTXO classifications (`Fee`, `Inscription`, `Leftover`, `Spent`).
    /// It's designed to be called at the end of an `inscribe` process or when
    /// it's necessary to refresh the UTXO set received from `Inscriber::get_utxos`, ensuring the
    /// the state operates with the most current UTXO information available.
    pub(crate) fn reset_utxo_vault(&mut self) {
        self.inner.clear();
        log::info!("UTXO vault has been reset.");
    }

    /// Sorts UTXOs by value in ascending order to optimize for fee deduction.
    /// This helps in accumulating smaller UTXOs first, leaving larger ones
    /// for future `inscribe` calls.
    fn sort_utxos(&mut self, utxos: Vec<Utxo>) -> Vec<Utxo> {
        let mut all_utxos = Vec::new();
        all_utxos.extend(utxos);
        all_utxos.sort_unstable_by_key(|utxo| utxo.value);
        all_utxos
    }

    /// Classifies a new UTXO and adds it to the state.
    fn classify_utxo(&mut self, utxo: &Utxo, purpose: UtxoType, value: Amount) {
        self.inner.push(UtxoManager {
            utxo: utxo.clone(),
            purpose,
            value,
        });
    }

    #[cfg(test)]
    fn select_utxos(&self, purpose: UtxoType) -> Vec<&Utxo> {
        self.inner
            .iter()
            .filter_map(|utxo_manager| {
                (utxo_manager.purpose == purpose).then_some(&utxo_manager.utxo)
            })
            .collect()
    }

    #[cfg(test)]
    fn update_utxo_purpose(&mut self, utxo_id: &str, new_purpose: UtxoType) {
        if let Some(utxo) = self.inner.iter_mut().find(|c_utxo| {
            let txid_hex = hex::encode(c_utxo.utxo.outpoint.txid.clone());
            // Create a unique identifier for each UTXO in the format "txid_hex:vout"
            let txid_vout = format!("{}:{}", txid_hex, c_utxo.utxo.outpoint.vout);
            txid_vout == utxo_id
        }) {
            utxo.purpose = new_purpose;
            log::info!("UTXO updated: {:?}", utxo);
        } else {
            log::warn!("UTXO not found for updating: {}", utxo_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use ic_exports::ic_cdk::api::management_canister::bitcoin::Outpoint;
    use ord_rs::constants::POSTAGE;

    use super::*;

    fn get_mock_utxo(txid: &[u8; 32], vout: u32, value: u64) -> Utxo {
        Utxo {
            outpoint: Outpoint {
                txid: txid.to_vec(),
                vout,
            },
            value,
            height: 100,
        }
    }

    fn get_mock_utxos() -> Vec<Utxo> {
        vec![
            Utxo {
                outpoint: Outpoint {
                    txid: vec![0; 32],
                    vout: 0,
                },
                value: 50000, // satoshis
                height: 101,
            },
            Utxo {
                outpoint: Outpoint {
                    txid: vec![1; 32],
                    vout: 1,
                },
                value: 100000, // satoshis
                height: 102,
            },
        ]
    }

    #[test]
    fn process_utxos_enough_funds_for_fees_and_inscriptions() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 15000,
            reveal_fee: 10000,
            ..Default::default()
        };
        let required_sum = fees.postage + fees.commit_fee + fees.reveal_fee;

        let fetched_utxos = get_mock_utxos();

        let classified_utxos = utxo_store.process_utxos(fees, fetched_utxos).unwrap();

        // Expect that at least one UTXO is reserved for inscription
        assert!(!classified_utxos.is_empty());
        let total_spent = classified_utxos.iter().map(|utxo| utxo.value).sum::<u64>();
        assert!(total_spent >= required_sum);
    }

    #[test]
    fn process_utxos_just_enough_funds_for_inscriptions() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 100000,
            reveal_fee: 49667,
            ..Default::default()
        };
        let required_sum = fees.postage + fees.commit_fee + fees.reveal_fee;

        let fetched_utxos = get_mock_utxos();

        let classified_utxos = utxo_store.process_utxos(fees, fetched_utxos).unwrap();

        // Expect that at least one UTXO is reserved for inscription
        assert!(!classified_utxos.is_empty());
        let total_spent = classified_utxos.iter().map(|utxo| utxo.value).sum::<u64>();
        assert_eq!(total_spent, required_sum);
    }

    #[test]
    fn process_utxos_insufficient_funds_for_fees() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 250000,
            reveal_fee: 100000,
            ..Default::default()
        };

        let fetched_utxos = get_mock_utxos();

        let classified_utxos = utxo_store.process_utxos(fees, fetched_utxos);

        assert!(matches!(
            classified_utxos,
            Err(InscribeError::InsufficientFundsForFees(_))
        ));
    }

    #[test]
    fn update_utxo_purpose_after_use() {
        let mut utxo_store = UtxoStore::default();

        let fetched_utxos = get_mock_utxos();
        for utxo in fetched_utxos.into_iter() {
            utxo_store.classify_utxo(&utxo, UtxoType::Fee, Amount::from_sat(utxo.value));
        }

        let txid_hex = hex::encode([0; 32]);
        let utxo_id = format!("{}:0", txid_hex);
        utxo_store.update_utxo_purpose(&utxo_id, UtxoType::Inscription);

        let updated_utxo = utxo_store
            .inner
            .iter()
            .find(|um| {
                let txid_hex = hex::encode(um.utxo.outpoint.txid.clone());
                format!("{}:{}", txid_hex, um.utxo.outpoint.vout) == utxo_id
            })
            .expect("UTXO should exist");

        assert_eq!(updated_utxo.purpose, UtxoType::Inscription);
    }

    #[test]
    fn process_utxos_with_all_funds_dedicated_to_fees() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 50000,
            reveal_fee: 50000,
            ..Default::default()
        };

        let fetched_utxos = vec![get_mock_utxo(&[2; 32], 1, 100000)];

        let classified_utxos = utxo_store.process_utxos(fees, fetched_utxos);

        assert!(matches!(
            classified_utxos,
            Err(InscribeError::InsufficientFundsForFees(_))
        ));
    }

    #[test]
    fn process_utxos_one_leftover_utxo_properly_allocated() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 15000,
            reveal_fee: 15000,
            ..Default::default()
        };

        // because UTXOs are sorted in ascending order based on value,
        // we expect 2 UTXOs selected for the inscription.
        let fetched_utxos = vec![
            get_mock_utxo(&[4; 32], 3, 30333),
            get_mock_utxo(&[5; 32], 4, 50000),
            get_mock_utxo(&[6; 32], 5, 25000),
        ];

        let utxos_for_inscription = utxo_store.process_utxos(fees, fetched_utxos).unwrap();
        let leftover_utxos = utxo_store.select_utxos(UtxoType::Leftover);

        assert!(
            !utxos_for_inscription.is_empty(),
            "Should have UTXOs left for inscription"
        );
        assert_eq!(
            utxos_for_inscription.len(),
            2,
            "Expected 2 UTXOs to be selected"
        );

        assert_eq!(
            leftover_utxos.len(),
            1,
            "Expected 1 UTXO to be classified as `Leftover`"
        );
    }

    #[test]
    fn insufficient_funds_for_any_transaction() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 100000,
            reveal_fee: 100000,
            ..Default::default()
        };

        let fetched_utxos = vec![get_mock_utxo(&[7; 32], 6, 40000)];

        assert!(utxo_store.process_utxos(fees, fetched_utxos).is_err());
    }

    #[test]
    fn multiple_utxos_exact_funds_for_fees_and_postage() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 20000,
            reveal_fee: 20000,
            ..Default::default()
        };

        let fetched_utxos = vec![
            get_mock_utxo(&[8; 32], 7, POSTAGE),
            get_mock_utxo(&[9; 32], 8, 20000),
            get_mock_utxo(&[10; 32], 9, 20000),
        ];

        assert!(utxo_store.process_utxos(fees, fetched_utxos).is_ok());
    }

    #[test]
    fn process_utxos_two_leftover_utxos_properly_allocated() {
        let mut utxo_store = UtxoStore::default();

        let fees = InscriptionFees {
            postage: POSTAGE,
            commit_fee: 15000,
            reveal_fee: 15000,
            ..Default::default()
        };

        // because UTXOs are sorted in ascending order based on value,
        // we expect 1 UTXO selected for the inscription.
        let fetched_utxos = vec![
            get_mock_utxo(&[4; 32], 3, 30333),
            get_mock_utxo(&[5; 32], 4, 50000),
            get_mock_utxo(&[6; 32], 5, 60000),
        ];

        let utxos_for_inscription = utxo_store.process_utxos(fees, fetched_utxos).unwrap();
        let leftover_utxos = utxo_store.select_utxos(UtxoType::Leftover);

        assert!(
            !utxos_for_inscription.is_empty(),
            "Should have 1 UTXO left for inscription"
        );
        assert_eq!(
            utxos_for_inscription.len(),
            1,
            "Expected 1 UTXO to be selected"
        );

        assert_eq!(
            leftover_utxos.len(),
            2,
            "Expected 2 UTXOs to be classified as `Leftover`"
        );
    }
}
