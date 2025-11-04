//! Type checking system for OtterLang
//!
//! Provides type inference, validation, and error reporting

pub mod checker;
pub mod types;

pub use checker::TypeChecker;
pub use types::{TypeContext, TypeError, TypeInfo};
