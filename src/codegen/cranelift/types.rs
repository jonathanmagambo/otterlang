use anyhow::Result;
use ast::nodes::{Function, Type};
use cranelift_codegen::ir;

use crate::typecheck::TypeInfo;

use super::backend::CraneliftBackend;

impl CraneliftBackend {
    fn type_info_to_cranelift(&self, type_info: &TypeInfo) -> Result<ir::Type> {
        match type_info {
            TypeInfo::Unit => Ok(ir::types::INVALID), // Void/unit type
            TypeInfo::Bool => Ok(ir::types::I8),
            TypeInfo::I32 => Ok(ir::types::I32),
            TypeInfo::I64 => Ok(ir::types::I64),
            TypeInfo::F64 => Ok(ir::types::F64),
            TypeInfo::Str => Ok(self.module.target_config().pointer_type()), // String as pointer
            TypeInfo::List(_) | TypeInfo::Dict { .. } => {
                // Collections are represented as pointers for now
                Ok(self.module.target_config().pointer_type())
            }
            TypeInfo::Function { .. } => {
                // Function pointers
                Ok(self.module.target_config().pointer_type())
            }
            TypeInfo::Struct { .. } | TypeInfo::Enum { .. } => {
                // Custom types are represented as pointers
                // In a full implementation, we might want to handle structs differently
                Ok(self.module.target_config().pointer_type())
            }
            TypeInfo::Generic { base, args } => {
                // Handle generic types based on base type
                match base.as_str() {
                    "List" | "list" => Ok(self.module.target_config().pointer_type()),
                    "Dict" | "dict" | "Map" | "map" => {
                        Ok(self.module.target_config().pointer_type())
                    }
                    _ => {
                        // For custom generic types, use pointer representation
                        Ok(self.module.target_config().pointer_type())
                    }
                }
            }
            TypeInfo::Alias { underlying, .. } => {
                // Follow the alias to the underlying type
                self.type_info_to_cranelift(underlying)
            }
            TypeInfo::Unknown => {
                // Unknown types default to pointer size
                Ok(self.module.target_config().pointer_type())
            }
            TypeInfo::Error => {
                // Error types are represented as strings
                Ok(self.module.target_config().pointer_type())
            }
            TypeInfo::Module(_) => {
                // Module types are opaque pointers
                Ok(self.module.target_config().pointer_type())
            }
        }
    }

    /// Convert Otter AST Type to Cranelift type (legacy method for backward compatibility)
    fn otter_type_to_cranelift(&self, otter_type: &Type) -> Result<ir::Type> {
        // Convert AST Type to TypeInfo first, then to Cranelift type
        let type_info = TypeInfo::from(otter_type);
        self.type_info_to_cranelift(&type_info)
    }

    /// Build a function signature from Otter function
    fn build_signature(&self, function: &Function) -> Result<ir::Signature> {
        let mut sig = ir::Signature::new(self.isa.default_call_conv());

        // Add parameters
        for param in &function.params {
            if let Some(ty) = &param.ty {
                let param_type = self.otter_type_to_cranelift(ty)?;
                sig.params.push(ir::AbiParam::new(param_type));
            } else {
                // Default to i64 if no type specified
                sig.params.push(ir::AbiParam::new(ir::types::I64));
            }
        }

        // Add return type
        let ret_type = if let Some(ret_ty) = &function.ret_ty {
            self.otter_type_to_cranelift(ret_ty)?
        } else {
            ir::types::I64 // Default return type
        };

        if ret_type != ir::types::INVALID {
            sig.returns.push(ir::AbiParam::new(ret_type));
        }

        Ok(sig)
    }
}
