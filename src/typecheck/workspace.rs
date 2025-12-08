use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::checker::{ModuleExports, TypeChecker};
use crate::typecheck::types::{EnumLayout, TypeError, TypeInfo};
use otterc_ast::nodes::{Program, Statement};
use otterc_language::LanguageFeatureFlags;
use otterc_span::Span;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleDependency {
    pub module: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModuleRecord {
    pub program: Program,
    pub exports: ModuleExports,
    pub diagnostics: Vec<TypeError>,
    pub expr_types: HashMap<usize, TypeInfo>,
    pub span_types: HashMap<Span, TypeInfo>,
    pub comprehension_types: HashMap<Span, TypeInfo>,
    pub enum_layouts: HashMap<String, EnumLayout>,
    pub dependencies: Vec<ModuleDependency>,
}

pub struct TypecheckWorkspace {
    features: LanguageFeatureFlags,
    registry: Option<&'static SymbolRegistry>,
    modules: HashMap<String, ModuleRecord>,
}

impl TypecheckWorkspace {
    pub fn new() -> Self {
        Self::with_features(LanguageFeatureFlags::default())
    }

    pub fn with_features(features: LanguageFeatureFlags) -> Self {
        Self {
            features,
            registry: None,
            modules: HashMap::new(),
        }
    }

    pub fn with_registry(mut self, registry: &'static SymbolRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn analyze_module(
        &mut self,
        module: impl Into<String>,
        program: Program,
    ) -> Result<&ModuleRecord> {
        let module_id = module.into();
        let dependencies = Self::collect_dependencies(&program);

        let mut checker = TypeChecker::with_language_features(self.features.clone());
        if let Some(registry) = self.registry {
            checker = checker.with_registry(registry);
        }

        for dependency in &dependencies {
            let alias = dependency
                .alias
                .as_deref()
                .unwrap_or_else(|| dependency.module.as_str());
            if let Some(dep_record) = self.modules.get(&dependency.module) {
                checker.import_module_exports(alias, &dep_record.exports);
            }
        }

        let check_result = checker.check_program(&program);
        let exports = checker.collect_public_exports(&module_id, &program);
        let diagnostics = checker.errors().to_vec();
        let enum_layouts = checker.enum_layouts();
        let (expr_types, span_types, comprehension_types) = checker.into_type_maps();

        let record = ModuleRecord {
            program,
            exports,
            diagnostics,
            expr_types,
            span_types,
            comprehension_types,
            enum_layouts,
            dependencies,
        };

        self.modules.insert(module_id.clone(), record);
        let entry = self.modules.get(&module_id).unwrap();
        if let Err(err) = check_result {
            Err(err)
        } else {
            Ok(entry)
        }
    }

    pub fn module(&self, name: &str) -> Option<&ModuleRecord> {
        self.modules.get(name)
    }

    pub fn modules(&self) -> impl Iterator<Item = (&String, &ModuleRecord)> {
        self.modules.iter()
    }

    pub fn module_ids(&self) -> impl Iterator<Item = &String> {
        self.modules.keys()
    }

    pub fn diagnostics(&self, name: &str) -> Option<&[TypeError]> {
        self.modules
            .get(name)
            .map(|record| record.diagnostics.as_slice())
    }

    fn collect_dependencies(program: &Program) -> Vec<ModuleDependency> {
        let mut seen = HashSet::new();
        let mut deps = Vec::new();
        for statement in &program.statements {
            match statement.as_ref() {
                Statement::Use { imports } => {
                    for import in imports {
                        let module = import.as_ref().module.clone();
                        let alias = import.as_ref().alias.clone();
                        let key = (module.clone(), alias.clone());
                        if seen.insert(key) {
                            deps.push(ModuleDependency { module, alias });
                        }
                    }
                }
                Statement::PubUse { module, alias, .. } => {
                    let key = (module.clone(), alias.clone());
                    if seen.insert(key) {
                        deps.push(ModuleDependency {
                            module: module.clone(),
                            alias: alias.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        deps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otterc_ast::nodes::{
        BinaryOp, Block, Expr, Function, Literal, Node, NumberLiteral, Param, Program, Statement,
        Type, UseImport,
    };

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn literal_int(value: i64) -> Node<Expr> {
        Node::new(
            Expr::Literal(Node::new(
                Literal::Number(NumberLiteral::new(value as f64, false)),
                span(),
            )),
            span(),
        )
    }

    #[test]
    fn workspace_produces_per_module_diagnostics() {
        let mut math_fn = Function::new(
            "add_one",
            vec![Node::new(
                Param::new(
                    Node::new("value".to_string(), span()),
                    Some(Node::new(Type::Simple("int".into()), span())),
                    None,
                ),
                span(),
            )],
            Some(Node::new(Type::Simple("int".into()), span())),
            Node::new(
                Block {
                    statements: vec![Node::new(
                        Statement::Return(Some(Node::new(
                            Expr::Binary {
                                op: BinaryOp::Add,
                                left: Box::new(Node::new(
                                    Expr::Identifier("value".to_string()),
                                    span(),
                                )),
                                right: Box::new(literal_int(1)),
                            },
                            span(),
                        ))),
                        span(),
                    )],
                },
                span(),
            ),
        );
        math_fn.public = true;
        let math_program = Program::new(vec![Node::new(
            Statement::Function(Node::new(math_fn, span())),
            span(),
        )]);

        let mut workspace = TypecheckWorkspace::new();
        workspace
            .analyze_module("math", math_program)
            .expect("math module should type-check");

        let use_stmt = Statement::Use {
            imports: vec![Node::new(UseImport::new("math", None), span())],
        };
        let call_expr = Node::new(
            Expr::Call {
                func: Box::new(Node::new(
                    Expr::Member {
                        object: Box::new(Node::new(Expr::Identifier("math".to_string()), span())),
                        field: "add_one".to_string(),
                    },
                    span(),
                )),
                args: vec![literal_int(41)],
            },
            span(),
        );
        let mut entry_fn = Function::new(
            "main",
            Vec::new(),
            Some(Node::new(Type::Simple("int".into()), span())),
            Node::new(
                Block {
                    statements: vec![Node::new(Statement::Return(Some(call_expr)), span())],
                },
                span(),
            ),
        );
        entry_fn.public = true;
        let app_program = Program::new(vec![
            Node::new(use_stmt, span()),
            Node::new(Statement::Function(Node::new(entry_fn, span())), span()),
        ]);

        let result = workspace.analyze_module("app", app_program);
        assert!(
            result.is_ok(),
            "app module should pass type checking: {:?}",
            result
        );
        let record = workspace.module("app").unwrap();
        assert!(record.diagnostics.is_empty());
        assert!(record.exports.functions.contains_key("main"));
    }
}
