use super::{RuntimeConstant, RuntimeType};
use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// Key for identifying specialized function versions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpecializationKey {
    pub function_name: String,
    pub arg_types: Vec<RuntimeType>,
    pub arg_constants: Vec<Option<RuntimeConstant>>,
}

impl SpecializationKey {
    pub fn new(
        function_name: String,
        arg_types: Vec<RuntimeType>,
        arg_constants: Vec<Option<RuntimeConstant>>,
    ) -> Self {
        Self {
            function_name,
            arg_types,
            arg_constants,
        }
    }

    pub fn hash_key(&self) -> u64 {
        let mut hasher = AHasher::default();
        self.hash(&mut hasher);
        hasher.finish()
    }
}
