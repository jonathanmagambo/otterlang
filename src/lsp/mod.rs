use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::{self, TypeChecker};
use ast::nodes::{Expr, Program, Statement};
use common::Span;
use lexer::{tokenize, LexerError, Token};
use parser::parse;
use utils::errors::{Diagnostic as OtterDiagnostic, DiagnosticSeverity as OtterDiagSeverity};

#[derive(Debug, Clone)]
struct SymbolInfo {
    span: Span,
    kind: SymbolKind,
    ty: Option<String>,
}

#[derive(Debug, Clone)]
enum SymbolKind {
    Variable,
    Parameter,
    Function,
    Struct,
    Enum,
    TypeAlias,
    #[allow(dead_code)]
    Method,
}

/// Symbol table mapping names to their definition locations and metadata
#[derive(Debug, Clone, Default)]
struct SymbolTable {
    /// All symbols with their info
    symbols: HashMap<String, SymbolInfo>,
    /// References: symbol name -> list of spans where it's used
    references: HashMap<String, Vec<Span>>,
}

impl SymbolTable {
    fn new() -> Self {
        Self::default()
    }

    fn add_variable(&mut self, name: String, span: Span, ty: Option<String>) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Variable,
                ty,
            },
        );
    }

    fn add_parameter(&mut self, name: String, span: Span, ty: Option<String>) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Parameter,
                ty,
            },
        );
    }

    fn add_function(&mut self, name: String, span: Span, ty: Option<String>) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Function,
                ty,
            },
        );
    }

    fn add_struct(&mut self, name: String, span: Span) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Struct,
                ty: None,
            },
        );
    }

    fn add_enum(&mut self, name: String, span: Span) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Enum,
                ty: None,
            },
        );
    }

    fn add_type_alias(&mut self, name: String, span: Span) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::TypeAlias,
                ty: None,
            },
        );
    }

    fn add_reference(&mut self, name: String, span: Span) {
        self.references
            .entry(name)
            .or_default()
            .push(span);
    }

    fn find_definition(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(name)
    }

    fn find_references(&self, name: &str) -> &[Span] {
        self.references
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn all_symbols(&self) -> impl Iterator<Item = (&String, &SymbolInfo)> {
        self.symbols.iter()
    }
}

#[derive(Default, Debug)]
struct DocumentStore {
    documents: HashMap<Url, String>,
    symbol_tables: HashMap<Url, SymbolTable>,
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    state: Arc<RwLock<DocumentStore>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(RwLock::new(DocumentStore::default())),
        }
    }

    async fn upsert_document(&self, uri: Url, text: String) {
        {
            let mut state = self.state.write().await;
            state.documents.insert(uri.clone(), text);
        }
        self.publish_diagnostics(uri).await;
    }

    async fn remove_document(&self, uri: &Url) {
        {
            let mut state = self.state.write().await;
            state.documents.remove(uri);
        }
        let _ = self
            .client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;
    }

    async fn publish_diagnostics(&self, uri: Url) {
        let text = {
            let state = self.state.read().await;
            state.documents.get(&uri).cloned()
        };

        if let Some(text) = text {
            let (diagnostics, symbol_table) = compute_lsp_diagnostics_and_symbols(&text);

            // Store the symbol table
            {
                let mut state = self.state.write().await;
                state.symbol_tables.insert(uri.clone(), symbol_table);
            }

            let _ = self
                .client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
    }

    #[allow(dead_code)]
    async fn document_text(&self, uri: &Url) -> Option<String> {
        let state = self.state.read().await;
        state.documents.get(uri).cloned()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    resolve_provider: Some(true),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        resolve_provider: Some(true),
                        work_done_progress_options: Default::default(),
                    },
                ))),
                semantic_tokens_provider: Some(
                    SemanticTokensOptions {
                        legend: SemanticTokensLegend {
                            token_types: vec![
                                SemanticTokenType::FUNCTION,
                                SemanticTokenType::VARIABLE,
                                SemanticTokenType::PARAMETER,
                                SemanticTokenType::TYPE,
                                SemanticTokenType::CLASS,
                                SemanticTokenType::ENUM,
                            ],
                            token_modifiers: vec![],
                        },
                        range: Some(true),
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        work_done_progress_options: Default::default(),
                    }
                    .into(),
                ),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "otterlang-lsp initialized")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.upsert_document(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.upsert_document(params.text_document.uri, change.text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.remove_document(&params.text_document.uri).await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table)
            && let Some(var_name) = word_at_position(&text, position)
                && let Some(symbol_info) = symbol_table.find_definition(&var_name) {
                    let range = span_to_range(symbol_info.span, &text);
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: uri.clone(),
                        range,
                    })));
                }

        Ok(None)
    }

    async fn goto_type_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        // For now, same as goto_definition
        self.goto_definition(params).await
    }

    async fn goto_implementation(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        // For now, same as goto_definition
        self.goto_definition(params).await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table)
            && let Some(var_name) = word_at_position(&text, position) {
                let mut locations = Vec::new();

                // Add definition
                if let Some(symbol_info) = symbol_table.find_definition(&var_name) {
                    locations.push(Location {
                        uri: uri.clone(),
                        range: span_to_range(symbol_info.span, &text),
                    });
                }

                // Add all references
                for span in symbol_table.find_references(&var_name) {
                    locations.push(Location {
                        uri: uri.clone(),
                        range: span_to_range(*span, &text),
                    });
                }

                return Ok(Some(locations));
            }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table) {
            let mut symbols = Vec::new();
            for (name, info) in symbol_table.all_symbols() {
                let kind = match info.kind {
                    SymbolKind::Function => tower_lsp::lsp_types::SymbolKind::FUNCTION,
                    SymbolKind::Variable => tower_lsp::lsp_types::SymbolKind::VARIABLE,
                    SymbolKind::Parameter => tower_lsp::lsp_types::SymbolKind::VARIABLE,
                    SymbolKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
                    SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
                    SymbolKind::TypeAlias => tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER,
                    SymbolKind::Method => tower_lsp::lsp_types::SymbolKind::METHOD,
                };
                #[allow(deprecated)]
                let symbol = DocumentSymbol {
                    name: name.clone(),
                    detail: info.ty.clone(),
                    kind,
                    range: span_to_range(info.span, &text),
                    selection_range: span_to_range(info.span, &text),
                    children: None,
                    deprecated: None,
                    tags: None,
                };
                symbols.push(symbol);
            }
            return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
        }

        Ok(None)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();
        let mut results = Vec::new();

        let state = self.state.read().await;
        for (uri, symbol_table) in &state.symbol_tables {
            if let Some(text) = state.documents.get(uri) {
                for (name, info) in symbol_table.all_symbols() {
                    if name.to_lowercase().contains(&query) {
                        let kind = match info.kind {
                            SymbolKind::Function => tower_lsp::lsp_types::SymbolKind::FUNCTION,
                            SymbolKind::Variable => tower_lsp::lsp_types::SymbolKind::VARIABLE,
                            SymbolKind::Parameter => tower_lsp::lsp_types::SymbolKind::VARIABLE,
                            SymbolKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
                            SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
                            SymbolKind::TypeAlias => {
                                tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER
                            }
                            SymbolKind::Method => tower_lsp::lsp_types::SymbolKind::METHOD,
                        };
                        #[allow(deprecated)]
                        let info = SymbolInformation {
                            name: name.clone(),
                            kind,
                            location: Location {
                                uri: uri.clone(),
                                range: span_to_range(info.span, text),
                            },
                            container_name: None,
                            deprecated: None,
                            tags: None,
                        };
                        results.push(info);
                    }
                }
            }
        }

        Ok(Some(results))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table)
            && let Some(old_name) = word_at_position(&text, position) {
                let mut changes = HashMap::new();
                let mut edits = Vec::new();

                // Add definition rename
                if let Some(symbol_info) = symbol_table.find_definition(&old_name) {
                    edits.push(TextEdit {
                        range: span_to_range(symbol_info.span, &text),
                        new_text: new_name.clone(),
                    });
                }

                // Add all references
                for span in symbol_table.find_references(&old_name) {
                    edits.push(TextEdit {
                        range: span_to_range(*span, &text),
                        new_text: new_name.clone(),
                    });
                }

                if !edits.is_empty() {
                    changes.insert(uri, edits);
                    return Ok(Some(WorkspaceEdit {
                        changes: Some(changes),
                        document_changes: None,
                        change_annotations: None,
                    }));
                }
            }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table)
            && let Some(var_name) = word_at_position(&text, position)
                && let Some(symbol_info) = symbol_table.find_definition(&var_name) {
                    let kind_str = match symbol_info.kind {
                        SymbolKind::Function => "function",
                        SymbolKind::Variable => "variable",
                        SymbolKind::Parameter => "parameter",
                        SymbolKind::Struct => "struct",
                        SymbolKind::Enum => "enum",
                        SymbolKind::TypeAlias => "type",
                        SymbolKind::Method => "method",
                    };
                    let detail = symbol_info
                        .ty
                        .as_ref()
                        .map(|ty| format!("{}: {}", kind_str, ty))
                        .unwrap_or_else(|| kind_str.to_string());

                    let contents = HoverContents::Scalar(MarkedString::String(detail));
                    return Ok(Some(Hover {
                        contents,
                        range: Some(span_to_range(symbol_info.span, &text)),
                    }));
                }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let _position = params.text_document_position.position;

        let (_text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        let mut items = Vec::new();

        // Add built-in functions
        items.push(CompletionItem {
            label: "print".into(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("def print(message: string)".into()),
            ..Default::default()
        });

        // Add symbols from symbol table
        if let Some(symbol_table) = symbol_table {
            for (name, info) in symbol_table.all_symbols() {
                let kind = match info.kind {
                    SymbolKind::Function => CompletionItemKind::FUNCTION,
                    SymbolKind::Variable => CompletionItemKind::VARIABLE,
                    SymbolKind::Parameter => CompletionItemKind::VARIABLE,
                    SymbolKind::Struct => CompletionItemKind::STRUCT,
                    SymbolKind::Enum => CompletionItemKind::ENUM,
                    SymbolKind::TypeAlias => CompletionItemKind::TYPE_PARAMETER,
                    SymbolKind::Method => CompletionItemKind::METHOD,
                };
                items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(kind),
                    detail: info.ty.clone(),
                    ..Default::default()
                });
            }
        }

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn inlay_hint(&self, _params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        Ok(Some(Vec::new()))
    }

    async fn inlay_hint_resolve(&self, hint: InlayHint) -> Result<InlayHint> {
        Ok(hint)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table) {
            let mut tokens = Vec::new();
            let mut prev_line = 0;
            let mut prev_col = 0;

            for (_name, info) in symbol_table.all_symbols() {
                let pos = span_to_position(info.span.start(), &text);
                let token_type = match info.kind {
                    SymbolKind::Function | SymbolKind::Method => 0, // FUNCTION
                    SymbolKind::Variable => 1,                      // VARIABLE
                    SymbolKind::Parameter => 2,                     // PARAMETER
                    SymbolKind::Struct => 4,                        // CLASS
                    SymbolKind::Enum => 5,                          // ENUM
                    SymbolKind::TypeAlias => 3,                     // TYPE
                };

                let delta_line = pos.line as u32 - prev_line;
                let delta_start = if delta_line == 0 {
                    pos.character as u32 - prev_col
                } else {
                    pos.character as u32
                };
                let length = (info.span.end() - info.span.start()) as u32;

                tokens.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length,
                    token_type,
                    token_modifiers_bitset: 0,
                });

                prev_line = pos.line as u32;
                prev_col = pos.character as u32;
            }

            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: tokens,
            })));
        }

        Ok(None)
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<Vec<CodeActionOrCommand>>> {
        let mut actions = Vec::new();

        // Add "Add type annotation" action for variables
        for diag in &params.context.diagnostics {
            if diag.message.contains("type") {
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Add type annotation".into(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diag.clone()]),
                    edit: None,
                    command: None,
                    is_preferred: Some(true),
                    disabled: None,
                    data: None,
                }));
            }
        }

        // Add "Extract function" action
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Extract function".into(),
            kind: Some(CodeActionKind::REFACTOR_EXTRACT),
            diagnostics: None,
            edit: None,
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }
}

/// Convert span start to Position
fn span_to_position(byte_offset: usize, text: &str) -> Position {
    let mut line = 0;
    let mut character = 0;

    for (i, ch) in text.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
    }

    Position { line, character }
}

/// Run a standard I/O LSP server using the backend above.
pub async fn run_stdio_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

/// Build symbol table from program, tracking definitions and references
fn build_symbol_table(program: &Program, tokens: &[Token], text: &str) -> SymbolTable {
    let mut table = SymbolTable::new();

    // First pass: collect all definitions
    build_symbol_table_from_statements(&program.statements, &mut table, tokens, text);

    // Second pass: collect references from expressions
    collect_references_from_statements(&program.statements, &mut table, tokens, text);

    table
}

/// Recursively extract symbol definitions from statements
fn build_symbol_table_from_statements(
    statements: &[Statement],
    table: &mut SymbolTable,
    tokens: &[Token],
    text: &str,
) {
    for stmt in statements {
        match stmt {
            Statement::Let {
                name, span, expr, ..
            } => {
                if let Some(span) = span {
                    let ty = infer_type_from_expr(expr);
                    table.add_variable(name.clone(), *span, ty);
                }
            }
            Statement::Function(func) => {
                // Find function name span from tokens
                if let Some(span) = find_name_span(&func.name, tokens, text) {
                    let sig = format_function_signature(func);
                    table.add_function(func.name.clone(), span, Some(sig));
                }
                for param in &func.params {
                    if let Some(span) = param.span {
                        let ty = param.ty.as_ref().map(format_type);
                        table.add_parameter(param.name.clone(), span, ty);
                    }
                }
                build_symbol_table_from_statements(&func.body.statements, table, tokens, text);
            }
            Statement::Struct { name, .. } => {
                if let Some(span) = find_name_span(name, tokens, text) {
                    table.add_struct(name.clone(), span);
                }
            }
            Statement::Enum { name, .. } => {
                if let Some(span) = find_name_span(name, tokens, text) {
                    table.add_enum(name.clone(), span);
                }
            }
            Statement::TypeAlias { name, .. } => {
                if let Some(span) = find_name_span(name, tokens, text) {
                    table.add_type_alias(name.clone(), span);
                }
            }
            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                build_symbol_table_from_statements(&then_block.statements, table, tokens, text);
                for (_, block) in elif_blocks {
                    build_symbol_table_from_statements(&block.statements, table, tokens, text);
                }
                if let Some(block) = else_block {
                    build_symbol_table_from_statements(&block.statements, table, tokens, text);
                }
            }
            Statement::For {
                var,
                var_span,
                body,
                ..
            } => {
                if let Some(span) = var_span {
                    table.add_variable(var.clone(), *span, None);
                }
                build_symbol_table_from_statements(&body.statements, table, tokens, text);
            }
            Statement::While { body, .. } => {
                build_symbol_table_from_statements(&body.statements, table, tokens, text);
            }
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
                ..
            } => {
                build_symbol_table_from_statements(&body.statements, table, tokens, text);
                for handler in handlers {
                    build_symbol_table_from_statements(
                        &handler.body.statements,
                        table,
                        tokens,
                        text,
                    );
                }
                if let Some(block) = else_block {
                    build_symbol_table_from_statements(&block.statements, table, tokens, text);
                }
                if let Some(block) = finally_block {
                    build_symbol_table_from_statements(&block.statements, table, tokens, text);
                }
            }
            Statement::Block(block) => {
                build_symbol_table_from_statements(&block.statements, table, tokens, text);
            }
            _ => {}
        }
    }
}

/// Collect references to symbols from expressions
fn collect_references_from_statements(
    statements: &[Statement],
    table: &mut SymbolTable,
    tokens: &[Token],
    text: &str,
) {
    for stmt in statements {
        match stmt {
            Statement::Function(func) => {
                collect_references_from_expr(
                    &Expr::Call {
                        func: Box::new(Expr::Identifier {
                            name: func.name.clone(),
                            span: None,
                        }),
                        args: vec![],
                    },
                    table,
                    tokens,
                    text,
                );
                collect_references_from_statements(&func.body.statements, table, tokens, text);
            }
            Statement::Let { expr, .. } => {
                collect_references_from_expr(expr, table, tokens, text);
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                collect_references_from_expr(cond, table, tokens, text);
                collect_references_from_statements(&then_block.statements, table, tokens, text);
                for (cond, block) in elif_blocks {
                    collect_references_from_expr(cond, table, tokens, text);
                    collect_references_from_statements(&block.statements, table, tokens, text);
                }
                if let Some(block) = else_block {
                    collect_references_from_statements(&block.statements, table, tokens, text);
                }
            }
            Statement::For { iterable, body, .. } => {
                collect_references_from_expr(iterable, table, tokens, text);
                collect_references_from_statements(&body.statements, table, tokens, text);
            }
            Statement::While { cond, body } => {
                collect_references_from_expr(cond, table, tokens, text);
                collect_references_from_statements(&body.statements, table, tokens, text);
            }
            Statement::Expr(expr) => {
                collect_references_from_expr(expr, table, tokens, text);
            }
            Statement::Return(Some(expr)) => {
                collect_references_from_expr(expr, table, tokens, text);
            }
            _ => {}
        }
    }
}

/// Collect references from an expression
fn collect_references_from_expr(
    expr: &Expr,
    table: &mut SymbolTable,
    tokens: &[Token],
    text: &str,
) {
    match expr {
        Expr::Identifier { name, .. } => {
            if let Some(span) = find_name_span(name, tokens, text) {
                table.add_reference(name.clone(), span);
            }
        }
        Expr::Call { func, args } => {
            collect_references_from_expr(func, table, tokens, text);
            for arg in args {
                collect_references_from_expr(arg, table, tokens, text);
            }
        }
        Expr::Member { object, .. } => {
            collect_references_from_expr(object, table, tokens, text);
        }
        Expr::Binary { left, right, .. } => {
            collect_references_from_expr(left, table, tokens, text);
            collect_references_from_expr(right, table, tokens, text);
        }
        Expr::Unary { expr, .. } => {
            collect_references_from_expr(expr, table, tokens, text);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_references_from_expr(cond, table, tokens, text);
            collect_references_from_expr(then_branch, table, tokens, text);
            if let Some(else_expr) = else_branch {
                collect_references_from_expr(else_expr, table, tokens, text);
            }
        }
        Expr::Array(elements) => {
            for elem in elements {
                collect_references_from_expr(elem, table, tokens, text);
            }
        }
        Expr::Dict(pairs) => {
            for (key, value) in pairs {
                collect_references_from_expr(key, table, tokens, text);
                collect_references_from_expr(value, table, tokens, text);
            }
        }
        _ => {}
    }
}

/// Find span of a name in tokens (approximate)
fn find_name_span(name: &str, tokens: &[Token], _text: &str) -> Option<Span> {
    for token in tokens {
        if let lexer::token::TokenKind::Identifier(ref id) = token.kind
            && id == name {
                return Some(token.span);
            }
    }
    None
}

/// Format function signature for display
fn format_function_signature(func: &ast::nodes::Function) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty_str =
                p.ty.as_ref()
                    .map(|t| format!(": {}", format_type(t)))
                    .unwrap_or_default();
            format!("{}{}", p.name, ty_str)
        })
        .collect();
    let ret_ty = func
        .ret_ty
        .as_ref()
        .map(|t| format!(" -> {}", format_type(t)))
        .unwrap_or_default();
    format!("def {}({}){}", func.name, params.join(", "), ret_ty)
}

/// Format type for display
fn format_type(ty: &ast::nodes::Type) -> String {
    match ty {
        ast::nodes::Type::Simple(name) => name.clone(),
        ast::nodes::Type::Generic { base, args } => {
            let args_str: Vec<String> = args.iter().map(format_type).collect();
            format!("{}<{}>", base, args_str.join(", "))
        }
    }
}

/// Infer type hint from expression (basic)
fn infer_type_from_expr(_expr: &Expr) -> Option<String> {
    None // Could be enhanced with type inference
}

/// Compute diagnostics and build symbol table from source text
fn compute_lsp_diagnostics_and_symbols(text: &str) -> (Vec<Diagnostic>, SymbolTable) {
    let source_id = "lsp";
    match tokenize(text) {
        Ok(tokens) => match parse(&tokens) {
            Ok(program) => {
                // Build symbol table from the parsed program
                let symbol_table = build_symbol_table(&program, &tokens, text);

                let diagnostics = {
                    let mut checker = TypeChecker::new().with_registry(SymbolRegistry::global());
                    if checker.check_program(&program).is_err() {
                        typecheck::diagnostics_from_type_errors(checker.errors(), source_id, text)
                            .into_iter()
                            .map(|diag| otter_diag_to_lsp(&diag, text))
                            .collect()
                    } else {
                        Vec::new()
                    }
                };

                (diagnostics, symbol_table)
            }
            Err(errors) => {
                let diagnostics = errors
                    .into_iter()
                    .map(|err| otter_diag_to_lsp(&err.to_diagnostic(source_id), text))
                    .collect();
                (diagnostics, SymbolTable::new())
            }
        },
        Err(errors) => {
            let diagnostics = errors
                .into_iter()
                .map(|err| otter_diag_to_lsp(&lexer_error_to_diag(source_id, &err), text))
                .collect();
            (diagnostics, SymbolTable::new())
        }
    }
}

fn word_at_position(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let mut idx = position.character as isize;
    if idx as usize >= chars.len() {
        idx = chars.len() as isize - 1;
    }
    while idx >= 0 && !chars[idx as usize].is_alphanumeric() && chars[idx as usize] != '_' {
        idx -= 1;
    }
    if idx < 0 {
        return None;
    }
    let start = {
        let mut s = idx as usize;
        while s > 0 && (chars[s - 1].is_alphanumeric() || chars[s - 1] == '_') {
            s -= 1;
        }
        s
    };
    let mut end = idx as usize;
    while end + 1 < chars.len() && (chars[end + 1].is_alphanumeric() || chars[end + 1] == '_') {
        end += 1;
    }
    Some(chars[start..=end].iter().collect())
}

#[allow(dead_code)]
fn collect_identifiers(text: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for token in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if token.len() > 1 && token.chars().next().is_some_and(|c| c.is_alphabetic()) {
            set.insert(token.to_string());
        }
    }
    set.into_iter().collect()
}

fn lexer_error_to_diag(source: &str, err: &LexerError) -> OtterDiagnostic {
    err.to_diagnostic(source)
}

fn otter_diag_to_lsp(diag: &OtterDiagnostic, text: &str) -> Diagnostic {
    let range = span_to_range(diag.span(), text);
    let mut message = diag.message().to_string();
    if let Some(suggestion) = diag.suggestion() {
        message.push_str(&format!("\nSuggestion: {}", suggestion));
    }
    if let Some(help) = diag.help() {
        message.push_str(&format!("\nHelp: {}", help));
    }

    Diagnostic {
        range,
        severity: Some(match diag.severity() {
            OtterDiagSeverity::Error => DiagnosticSeverity::ERROR,
            OtterDiagSeverity::Warning => DiagnosticSeverity::WARNING,
            OtterDiagSeverity::Info => DiagnosticSeverity::INFORMATION,
            OtterDiagSeverity::Hint => DiagnosticSeverity::HINT,
        }),
        code: None,
        code_description: None,
        source: Some("otterlang".into()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn span_to_range(span: Span, text: &str) -> Range {
    Range {
        start: offset_to_position(text, span.start()),
        end: offset_to_position(text, span.end()),
    }
}

fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut counted = 0usize;
    let mut line = 0u32;
    let mut character = 0u32;
    for ch in text.chars() {
        if counted >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }
        counted += ch.len_utf8();
    }
    Position { line, character }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_symbol_table() {
        let test_code = r#"
let x = 10
let y = 20

def add(a, b):
    let result = a + b
    return result

let sum = add(x, y)

for i in [1, 2, 3]:
    let doubled = i * 2
    print(doubled)
"#;

        match tokenize(test_code) {
            Ok(tokens) => match parse(&tokens) {
                Ok(program) => {
                    let symbol_table = build_symbol_table(&program, &tokens, test_code);

                    assert!(
                        symbol_table.find_definition("x").is_some(),
                        "Variable 'x' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("y").is_some(),
                        "Variable 'y' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("result").is_some(),
                        "Variable 'result' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("sum").is_some(),
                        "Variable 'sum' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("doubled").is_some(),
                        "Variable 'doubled' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("a").is_some(),
                        "Parameter 'a' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("b").is_some(),
                        "Parameter 'b' should be in symbol table"
                    );
                    assert!(
                        symbol_table.find_definition("i").is_some(),
                        "Loop variable 'i' should be in symbol table"
                    );

                    println!("✓ All symbol table tests passed!");
                    let vars: Vec<_> = symbol_table.all_symbols().map(|(k, _)| k.clone()).collect();
                    println!("  Symbols: {:?}", vars);
                }
                Err(errors) => {
                    panic!("Parsing failed: {:?}", errors);
                }
            },
            Err(errors) => {
                panic!("Tokenization failed: {:?}", errors);
            }
        }
    }

    #[test]
    fn test_find_definition() {
        let test_code = "let x = 10\nlet y = x + 5\n";

        match tokenize(test_code) {
            Ok(tokens) => match parse(&tokens) {
                Ok(program) => {
                    let symbol_table = build_symbol_table(&program, &tokens, test_code);

                    let x_info = symbol_table.find_definition("x");
                    assert!(x_info.is_some(), "Should find definition for 'x'");

                    let y_span = symbol_table.find_definition("y");
                    assert!(y_span.is_some(), "Should find definition for 'y'");

                    let z_span = symbol_table.find_definition("z");
                    assert!(z_span.is_none(), "Should not find definition for 'z'");
                }
                Err(errors) => {
                    panic!("Parsing failed: {:?}", errors);
                }
            },
            Err(errors) => {
                panic!("Tokenization failed: {:?}", errors);
            }
        }
    }
}
