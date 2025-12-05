use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValueEnum, PointerValue};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtterType {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    Opaque,               // For handles, pointers, etc.
    List(Box<OtterType>), // Now tracks element type
    Map,
    Struct(u32),
    Tuple(Vec<OtterType>),
}

impl OtterType {
    /// Construct a list type that tracks its element type
    pub fn list_of(element: OtterType) -> Self {
        OtterType::List(Box::new(element))
    }

    /// Construct a list type when the element type is unknown/opaque
    pub fn opaque_list() -> Self {
        Self::list_of(OtterType::Opaque)
    }

    /// Get the element type carried by the list, if available
    pub fn list_element(&self) -> Option<&OtterType> {
        match self {
            OtterType::List(inner) => Some(inner.as_ref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvaluatedValue<'ctx> {
    pub ty: OtterType,
    pub value: Option<BasicValueEnum<'ctx>>,
}

impl<'ctx> EvaluatedValue<'ctx> {
    pub fn with_value(value: BasicValueEnum<'ctx>, ty: OtterType) -> Self {
        Self {
            ty,
            value: Some(value),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Variable<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub ty: OtterType,
}

#[derive(Debug, Clone)]
pub struct LoopContext<'ctx> {
    pub cond_bb: BasicBlock<'ctx>,
    pub exit_bb: BasicBlock<'ctx>,
}

#[derive(Debug, Clone)]
pub struct FunctionContext<'ctx> {
    pub variables: HashMap<String, Variable<'ctx>>,
    pub loop_stack: Vec<LoopContext<'ctx>>,
    pub exception_landingpad: Option<BasicBlock<'ctx>>,
}

impl<'ctx> FunctionContext<'ctx> {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            loop_stack: Vec::new(),
            exception_landingpad: None,
        }
    }

    pub fn insert(&mut self, name: String, var: Variable<'ctx>) {
        self.variables.insert(name, var);
    }

    pub fn get(&self, name: &str) -> Option<&Variable<'ctx>> {
        self.variables.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<Variable<'ctx>> {
        self.variables.remove(name)
    }

    pub fn push_loop(&mut self, cond_bb: BasicBlock<'ctx>, exit_bb: BasicBlock<'ctx>) {
        self.loop_stack.push(LoopContext { cond_bb, exit_bb });
    }

    pub fn pop_loop(&mut self) -> Option<LoopContext<'ctx>> {
        self.loop_stack.pop()
    }

    pub fn current_loop(&self) -> Option<&LoopContext<'ctx>> {
        self.loop_stack.last()
    }
}

impl<'ctx> Default for FunctionContext<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}
