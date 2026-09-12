//! Map from `LockType` to the module that serves it. First-party modules
//! are registered by `with_first_party`; tests and future extension hosts
//! register their own.

use std::collections::BTreeMap;

use lantern_sdk_schema::{LockModule, LockType};
use lantern_signer_secp256k1::Secp256k1Lock;

use crate::error::CoreError;

/// Registered lock modules keyed by `LockType`.
#[derive(Default)]
pub struct LockRegistry {
    modules: BTreeMap<LockType, Box<dyn LockModule>>,
}

impl LockRegistry {
    /// Empty registry. Use `with_first_party` for the shipping set.
    pub const fn new() -> Self {
        Self {
            modules: BTreeMap::new(),
        }
    }

    /// Registry with every first-party module Lantern ships in this plan.
    pub fn with_first_party() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(Secp256k1Lock));
        registry
    }

    /// Register or replace the module for its `lock_type`.
    pub fn register(&mut self, module: Box<dyn LockModule>) {
        self.modules.insert(module.lock_type(), module);
    }

    /// The module registered for `lock_type`, or `UnsupportedLock`.
    pub fn get(&self, lock_type: LockType) -> Result<&dyn LockModule, CoreError> {
        self.modules
            .get(&lock_type)
            .map(Box::as_ref)
            .ok_or(CoreError::UnsupportedLock(lock_type))
    }
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{LockType, WitnessSize};

    use super::LockRegistry;
    use crate::error::CoreError;

    #[test]
    fn first_party_registry_serves_secp256k1() {
        let registry = LockRegistry::with_first_party();
        let module = registry
            .get(LockType::Secp256k1Blake160)
            .expect("registered");
        assert_eq!(module.extension_id(), "core.secp256k1");
        assert_eq!(module.witness_size(), WitnessSize::Fixed(65));
    }

    #[test]
    fn empty_registry_reports_unsupported_lock() {
        let registry = LockRegistry::new();
        assert!(matches!(
            registry.get(LockType::Secp256k1Blake160),
            Err(CoreError::UnsupportedLock(LockType::Secp256k1Blake160))
        ));
    }
}
