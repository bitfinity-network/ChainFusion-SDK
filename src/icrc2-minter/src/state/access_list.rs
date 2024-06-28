use candid::Principal;
use ic_stable_structures::stable_structures::Memory;
use ic_stable_structures::{BTreeMapStructure, StableBTreeMap};

pub(crate) struct AccessList<M: Memory> {
    pub access_list: StableBTreeMap<Principal, (), M>,
}

impl<M: Memory> AccessList<M> {
    pub fn new(m: M) -> Self {
        Self {
            access_list: StableBTreeMap::new(m),
        }
    }
    pub fn add(&mut self, principal: Principal) -> minter_did::error::Result<()> {
        if principal == Principal::anonymous() {
            return Err(minter_did::error::Error::AnonymousPrincipal);
        }

        self.access_list.insert(principal, ());

        Ok(())
    }

    pub fn get_all_principals(&self) -> Vec<Principal> {
        self.access_list
            .iter()
            .map(|(principal, _)| principal)
            .collect()
    }

    pub fn remove(&mut self, principal: &Principal) {
        self.access_list.remove(principal);
    }
}

#[cfg(test)]
mod tests {
    use ic_exports::ic_kit::MockContext;

    use super::*;
    use crate::constant::ACCESS_LIST_MEMORY_ID;
    use crate::memory::MEMORY_MANAGER;

    impl<M: Memory> AccessList<M> {
        pub fn contains(&self, principal: &Principal) -> bool {
            self.access_list.contains_key(principal)
        }

        pub fn batch_add(&mut self, principals: &[Principal]) -> minter_did::error::Result<()> {
            for principal in principals {
                self.add(*principal)?;
            }

            Ok(())
        }
    }

    #[test]
    fn test_access_list() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principal = Principal::from_text("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap();
        access_list.add(principal).unwrap();
        assert!(access_list.contains(&principal));
    }

    #[test]
    fn test_access_list_batch() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principals = vec![
            Principal::management_canister(),
            Principal::from_text("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap(),
        ];
        access_list.batch_add(&principals).unwrap();
        assert!(access_list.contains(&principals[0]));
        assert!(access_list.contains(&principals[1]));
    }

    #[test]
    fn test_access_list_check() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principal = Principal::management_canister();
        access_list.add(principal).unwrap();
        assert!(access_list.contains(&principal));
        assert!(!access_list.contains(&Principal::anonymous()));
    }

    #[test]
    fn test_access_list_remove() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principal = Principal::management_canister();
        access_list.add(principal).unwrap();
        assert!(access_list.contains(&principal));
        access_list.remove(&principal);
        assert!(!access_list.contains(&principal));
    }

    #[test]
    fn test_access_list_remove_non_existing() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principal = Principal::management_canister();
        access_list.remove(&principal);
        assert!(!access_list.contains(&principal));
    }

    #[test]
    fn test_access_list_remove_multiple() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principals = vec![
            Principal::management_canister(),
            Principal::from_text("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap(),
        ];
        access_list.batch_add(&principals).unwrap();
        assert!(access_list.contains(&principals[0]));
        assert!(access_list.contains(&principals[1]));
        access_list.remove(&principals[0]);
        assert!(!access_list.contains(&principals[0]));
        assert!(access_list.contains(&principals[1]));

        assert_eq!(access_list.access_list.len(), 1);
    }

    #[test]
    fn test_access_list_get_all_principals() {
        MockContext::new().inject();

        let mut access_list =
            AccessList::new(MEMORY_MANAGER.with(|mm| mm.get(ACCESS_LIST_MEMORY_ID)));
        let principals = vec![
            Principal::management_canister(),
            Principal::from_text("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap(),
        ];
        access_list.batch_add(&principals).unwrap();
        let all_principals = access_list.get_all_principals();
        assert_eq!(all_principals.len(), 2);
        assert!(all_principals.contains(&principals[0]));
        assert!(all_principals.contains(&principals[1]));
    }
}
