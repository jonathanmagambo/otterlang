// JIT Specialization System
pub mod constant_prop;
pub mod key;
pub mod specializer;
pub mod type_tracker;

pub use constant_prop::ConstantPropagator;
pub use key::SpecializationKey;
pub use specializer::Specializer;
pub use type_tracker::TypeTracker;

use crate::runtime::symbol_registry::FfiType;
use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// Runtime type information for specialization
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuntimeType {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    Opaque,
    Unknown,
}

impl From<FfiType> for RuntimeType {
    fn from(ty: FfiType) -> Self {
        match ty {
            FfiType::Unit => RuntimeType::Unit,
            FfiType::Bool => RuntimeType::Bool,
            FfiType::I32 => RuntimeType::I32,
            FfiType::I64 => RuntimeType::I64,
            FfiType::F64 => RuntimeType::F64,
            FfiType::Str => RuntimeType::Str,
            FfiType::Opaque | FfiType::List | FfiType::Map => RuntimeType::Opaque,
        }
    }
}

/// Runtime constant value for specialization
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeConstant {
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Str(String),
}

impl RuntimeConstant {
    pub fn hash(&self) -> u64 {
        let mut hasher = AHasher::default();
        match self {
            RuntimeConstant::Bool(b) => b.hash(&mut hasher),
            RuntimeConstant::I32(i) => i.hash(&mut hasher),
            RuntimeConstant::I64(i) => i.hash(&mut hasher),
            RuntimeConstant::F64(f) => {
                // Approximate hash for floats
                f.to_bits().hash(&mut hasher)
            }
            RuntimeConstant::Str(s) => s.hash(&mut hasher),
        }
        hasher.finish()
    }
}

/// Represents a call site's specialization context
#[derive(Debug, Clone)]
pub struct CallSiteContext {
    pub function_name: String,
    pub arg_types: Vec<RuntimeType>,
    pub arg_constants: Vec<Option<RuntimeConstant>>,
}

impl CallSiteContext {
    pub fn new(function_name: String) -> Self {
        Self {
            function_name,
            arg_types: Vec::new(),
            arg_constants: Vec::new(),
        }
    }

    pub fn with_types(mut self, types: Vec<RuntimeType>) -> Self {
        let len = types.len();
        self.arg_types = types;
        self.arg_constants = vec![None; len];
        self
    }

    pub fn with_constants(mut self, constants: Vec<Option<RuntimeConstant>>) -> Self {
        self.arg_constants = constants;
        self
    }

    pub fn specialization_key(&self) -> SpecializationKey {
        let arg_types = self.arg_types.clone();
        let arg_constants = self.arg_constants.clone();
        SpecializationKey::new(self.function_name.clone(), arg_types, arg_constants)
    }
}
