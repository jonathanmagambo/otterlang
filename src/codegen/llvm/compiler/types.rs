use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValueEnum, PointerValue};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtterType {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    Opaque, // For handles, pointers, etc.
    List,
    Map,
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
