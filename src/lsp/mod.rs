use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::{self, TypeChecker};
use otterc_ast::nodes::{Expr, Function, Node, Program, Statement, Type};
use otterc_lexer::{LexerError, Token, tokenize};
use otterc_parser::parse;
use otterc_span::Span;
use otterc_utils::errors::{
    Diagnostic as OtterDiagnostic, DiagnosticSeverity as OtterDiagSeverity,
};

const BUILTIN_FUNCTION_COMPLETIONS: &[(&str, &str)] = &[
    ("print", "fn print(message: string) -> unit"),
    ("println", "fn println(message: string) -> unit"),
    ("eprintln", "fn eprintln(message: string) -> unit"),
    ("len", "fn len(collection: any) -> int"),
    ("cap", "fn cap(value: any) -> int"),
    ("append", "fn append(list: List, value: any) -> bool"),
    ("delete", "fn delete(map: Map, key: string) -> bool"),
    ("enumerate", "fn enumerate(list: List) -> List"),
    ("list_new", "fn list_new() -> List"),
    ("map_new", "fn map_new() -> Map"),
    ("range", "fn range(start: int, end: int) -> List"),
    (
        "range_float",
        "fn range_float(start: float, end: float) -> List",
    ),
    ("panic", "fn panic(message: string) -> unit"),
    ("recover", "fn recover() -> string"),
    ("type_of", "fn type_of(value: any) -> string"),
    ("fields", "fn fields(obj: any) -> string"),
    ("str", "fn str(value: any) -> string"),
];

const KEYWORD_COMPLETIONS: &[&str] = &[
    "fn", "let", "pub", "struct", "enum", "match", "case", "if", "elif", "else", "for", "while",
    "try", "except", "finally", "raise", "await", "spawn", "use", "from", "as", "type",
];

struct SnippetCompletion {
    label: &'static str,
    detail: &'static str,
    snippet: &'static str,
}

const SNIPPET_COMPLETIONS: &[SnippetCompletion] = &[
    SnippetCompletion {
        label: "fn snippet",
        detail: "Function definition",
        snippet: "fn ${1:name}(${2:params})${3: -> type}:\n    ${0}",
    },
    SnippetCompletion {
        label: "if/elif/else",
        detail: "Conditional block",
        snippet: "if ${1:condition}:\n    ${2}\nelif ${3:condition}:\n    ${4}\nelse:\n    ${0}",
    },
    SnippetCompletion {
        label: "match",
        detail: "Match expression",
        snippet: "match ${1:value}:\n    case ${2:Pattern}:\n        ${3}\n    case ${4:Pattern}:\n        ${0}",
    },
];

#[derive(Debug, Clone)]
struct SymbolInfo {
    span: Span,
    kind: SymbolKind,
    ty: Option<String>,
    callable: Option<CallableInfo>,
}

#[derive(Debug, Clone)]
struct CallableInfo {
    name: String,
    params: Vec<CallableParam>,
    return_type: Option<String>,
}

#[derive(Debug, Clone)]
struct CallableParam {
    name: String,
    ty: Option<String>,
    has_default: bool,
}

impl CallableInfo {
    fn from_function(func: &Function) -> Self {
        let params = func
            .params
            .iter()
            .map(|param| CallableParam {
                name: param.as_ref().name.as_ref().clone(),
                ty: param
                    .as_ref()
                    .ty
                    .as_ref()
                    .map(|ty| format_type(ty.as_ref())),
                has_default: param.as_ref().default.is_some(),
            })
            .collect();

        let return_type = func.ret_ty.as_ref().map(|ty| format_type(ty.as_ref()));

        Self {
            name: func.name.clone(),
            params,
            return_type,
        }
    }
}

#[derive(Debug, Clone)]
enum SymbolKind {
    Variable,
    Parameter,
    Function,
    Struct,
    Enum,
    TypeAlias,
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
                callable: None,
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
                callable: None,
            },
        );
    }

    fn add_function(
        &mut self,
        name: String,
        span: Span,
        ty: Option<String>,
        callable: Option<CallableInfo>,
    ) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Function,
                ty,
                callable,
            },
        );
    }

    fn add_method(
        &mut self,
        name: String,
        span: Span,
        ty: Option<String>,
        callable: Option<CallableInfo>,
    ) {
        self.symbols.insert(
            name.clone(),
            SymbolInfo {
                span,
                kind: SymbolKind::Method,
                ty,
                callable,
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
                callable: None,
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
                callable: None,
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
                callable: None,
            },
        );
    }

    fn add_reference(&mut self, name: String, span: Span) {
        self.references.entry(name).or_default().push(span);
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

    fn get(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(name)
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

    #[expect(dead_code, reason = "Work in progress")]
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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".into(), ",".into()]),
                    retrigger_characters: Some(vec![",".into()]),
                    work_done_progress_options: Default::default(),
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
            && let Some(symbol_info) = symbol_table.find_definition(&var_name)
        {
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
            && let Some(var_name) = word_at_position(&text, position)
        {
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
                    SymbolKind::Variable | SymbolKind::Parameter => {
                        tower_lsp::lsp_types::SymbolKind::VARIABLE
                    }
                    SymbolKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
                    SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
                    SymbolKind::TypeAlias => tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER,
                    SymbolKind::Method => tower_lsp::lsp_types::SymbolKind::METHOD,
                };
                #[expect(
                    deprecated,
                    reason = "We are not using this deprecated field but it's required for constructing DocumentSymbol"
                )]
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
                            SymbolKind::Variable | SymbolKind::Parameter => {
                                tower_lsp::lsp_types::SymbolKind::VARIABLE
                            }
                            SymbolKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
                            SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
                            SymbolKind::TypeAlias => {
                                tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER
                            }
                            SymbolKind::Method => tower_lsp::lsp_types::SymbolKind::METHOD,
                        };
                        #[expect(
                            deprecated,
                            reason = "We are not using this deprecated field but it's required for constructing SymbolInformation"
                        )]
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
            && let Some(old_name) = word_at_position(&text, position)
        {
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
            && let Some(symbol_info) = symbol_table.find_definition(&var_name)
        {
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

        for (label, detail) in BUILTIN_FUNCTION_COMPLETIONS {
            items.push(CompletionItem {
                label: (*label).into(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some((*detail).into()),
                ..Default::default()
            });
        }

        for keyword in KEYWORD_COMPLETIONS {
            items.push(CompletionItem {
                label: (*keyword).into(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("keyword".into()),
                ..Default::default()
            });
        }

        for snippet in SNIPPET_COMPLETIONS {
            items.push(CompletionItem {
                label: snippet.label.into(),
                kind: Some(CompletionItemKind::SNIPPET),
                detail: Some(snippet.detail.into()),
                insert_text: Some(snippet.snippet.into()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }

        // Add symbols from symbol table
        if let Some(symbol_table) = symbol_table {
            for (name, info) in symbol_table.all_symbols() {
                let kind = match info.kind {
                    SymbolKind::Function | SymbolKind::Variable | SymbolKind::Parameter => {
                        CompletionItemKind::VARIABLE
                    }
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

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let (text, symbol_table) = {
            let state = self.state.read().await;
            let text = state.documents.get(&uri).cloned();
            let symbol_table = state.symbol_tables.get(&uri).cloned();
            (text, symbol_table)
        };

        if let (Some(text), Some(symbol_table)) = (text, symbol_table) {
            let offset = position_to_offset(&text, position);
            if let Some((func_name, active_param)) = find_call_context(&text, offset)
                && let Some(symbol) = symbol_table.get(&func_name)
                && let Some(callable) = &symbol.callable
            {
                let parameters: Vec<ParameterInformation> = callable
                    .params
                    .iter()
                    .map(|param| {
                        let mut label = param.name.clone();
                        if let Some(ty) = &param.ty {
                            label.push_str(": ");
                            label.push_str(ty);
                        }
                        let mut doc_lines = Vec::new();
                        if let Some(ty) = &param.ty {
                            doc_lines.push(format!("type: {}", ty));
                        }
                        if param.has_default {
                            doc_lines.push("default parameter".to_string());
                        }
                        let documentation = if doc_lines.is_empty() {
                            None
                        } else {
                            Some(Documentation::String(doc_lines.join("\n")))
                        };
                        ParameterInformation {
                            label: ParameterLabel::Simple(label),
                            documentation,
                        }
                    })
                    .collect();

                let signature_label = symbol
                    .ty
                    .clone()
                    .unwrap_or_else(|| format_callable_signature(callable));

                let signature = SignatureInformation {
                    label: signature_label,
                    documentation: None,
                    parameters: Some(parameters.clone()),
                    active_parameter: None,
                };

                let active_param_index = if parameters.is_empty() {
                    0
                } else {
                    active_param.min(parameters.len() - 1)
                } as u32;

                return Ok(Some(SignatureHelp {
                    signatures: vec![signature],
                    active_signature: Some(0),
                    active_parameter: Some(active_param_index),
                }));
            }
        }

        Ok(None)
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
    statements: &[Node<Statement>],
    table: &mut SymbolTable,
    tokens: &[Token],
    text: &str,
) {
    for stmt in statements {
        let span = stmt.span();
        match stmt.as_ref() {
            Statement::Let { name, ty, expr, .. } => {
                let ty_str = ty
                    .as_ref()
                    .map(|ty| format_type(ty.as_ref()))
                    .or_else(|| infer_type_from_expr(expr.as_ref()));
                table.add_variable(name.as_ref().clone(), *span, ty_str);
            }

            Statement::Function(func) => {
                // Find function name span from tokens
                if let Some(span) = find_name_span(&func.as_ref().name, tokens, text) {
                    let sig = format_function_signature(func.as_ref());
                    let callable = Some(CallableInfo::from_function(func.as_ref()));
                    table.add_function(func.as_ref().name.clone(), span, Some(sig), callable);
                }
                for param in &func.as_ref().params {
                    let ty = param
                        .as_ref()
                        .ty
                        .as_ref()
                        .map(|ty| format_type(ty.as_ref()));
                    table.add_parameter(param.as_ref().name.as_ref().clone(), *param.span(), ty);
                }
                build_symbol_table_from_statements(
                    &func.as_ref().body.as_ref().statements,
                    table,
                    tokens,
                    text,
                );
            }
            Statement::Struct { name, methods, .. } => {
                if let Some(span) = find_name_span(name, tokens, text) {
                    table.add_struct(name.clone(), span);
                }
                for method in methods {
                    if let Some(span) = find_name_span(&method.as_ref().name, tokens, text) {
                        let sig = format_function_signature(method.as_ref());
                        let callable = Some(CallableInfo::from_function(method.as_ref()));
                        table.add_method(method.as_ref().name.clone(), span, Some(sig), callable);
                    }
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
                build_symbol_table_from_statements(
                    &then_block.as_ref().statements,
                    table,
                    tokens,
                    text,
                );
                for (_, block) in elif_blocks {
                    build_symbol_table_from_statements(
                        &block.as_ref().statements,
                        table,
                        tokens,
                        text,
                    );
                }
                if let Some(block) = else_block {
                    build_symbol_table_from_statements(
                        &block.as_ref().statements,
                        table,
                        tokens,
                        text,
                    );
                }
            }
            Statement::For { var, body, .. } => {
                table.add_variable(var.as_ref().clone(), *span, None);
                build_symbol_table_from_statements(&body.as_ref().statements, table, tokens, text);
            }
            Statement::While { body, .. } => {
                build_symbol_table_from_statements(&body.as_ref().statements, table, tokens, text);
            }
            Statement::Block(block) => {
                build_symbol_table_from_statements(&block.as_ref().statements, table, tokens, text);
            }
            _ => {}
        }
    }
}

/// Collect references to symbols from expressions
fn collect_references_from_statements(
    statements: &[Node<Statement>],
    table: &mut SymbolTable,
    tokens: &[Token],
    text: &str,
) {
    for stmt in statements {
        let span = stmt.span();
        match stmt.as_ref() {
            Statement::Function(func) => {
                collect_references_from_expr(
                    &Expr::Call {
                        func: Box::new(Node::new(
                            Expr::Identifier(func.as_ref().name.clone()),
                            *span,
                        )),
                        args: vec![],
                    },
                    table,
                    tokens,
                    text,
                );
                collect_references_from_statements(
                    &func.as_ref().body.as_ref().statements,
                    table,
                    tokens,
                    text,
                );
            }
            Statement::Let { expr, .. } | Statement::Expr(expr) | Statement::Return(Some(expr)) => {
                collect_references_from_expr(expr.as_ref(), table, tokens, text);
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                collect_references_from_expr(cond.as_ref(), table, tokens, text);
                collect_references_from_statements(
                    &then_block.as_ref().statements,
                    table,
                    tokens,
                    text,
                );
                for (cond, block) in elif_blocks {
                    collect_references_from_expr(cond.as_ref(), table, tokens, text);
                    collect_references_from_statements(
                        &block.as_ref().statements,
                        table,
                        tokens,
                        text,
                    );
                }
                if let Some(block) = else_block {
                    collect_references_from_statements(
                        &block.as_ref().statements,
                        table,
                        tokens,
                        text,
                    );
                }
            }
            Statement::For { iterable, body, .. } => {
                collect_references_from_expr(iterable.as_ref(), table, tokens, text);
                collect_references_from_statements(&body.as_ref().statements, table, tokens, text);
            }
            Statement::While { cond, body } => {
                collect_references_from_expr(cond.as_ref(), table, tokens, text);
                collect_references_from_statements(&body.as_ref().statements, table, tokens, text);
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
        Expr::Identifier(name) => {
            if let Some(span) = find_name_span(name, tokens, text) {
                table.add_reference(name.clone(), span);
            }
        }
        Expr::Call { func, args } => {
            collect_references_from_expr(func.as_ref().as_ref(), table, tokens, text);
            for arg in args {
                collect_references_from_expr(arg.as_ref(), table, tokens, text);
            }
        }
        Expr::Member { object, .. } => {
            collect_references_from_expr(object.as_ref().as_ref(), table, tokens, text);
        }
        Expr::Binary { left, right, .. } => {
            collect_references_from_expr(left.as_ref().as_ref(), table, tokens, text);
            collect_references_from_expr(right.as_ref().as_ref(), table, tokens, text);
        }
        Expr::Unary { expr, .. } => {
            collect_references_from_expr(expr.as_ref().as_ref(), table, tokens, text);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_references_from_expr(cond.as_ref().as_ref(), table, tokens, text);
            collect_references_from_expr(then_branch.as_ref().as_ref(), table, tokens, text);
            if let Some(else_expr) = else_branch {
                collect_references_from_expr(else_expr.as_ref().as_ref(), table, tokens, text);
            }
        }
        Expr::Array(elements) => {
            for elem in elements {
                collect_references_from_expr(elem.as_ref(), table, tokens, text);
            }
        }
        Expr::Dict(pairs) => {
            for (key, value) in pairs {
                collect_references_from_expr(key.as_ref(), table, tokens, text);
                collect_references_from_expr(value.as_ref(), table, tokens, text);
            }
        }
        _ => {}
    }
}

/// Find span of a name in tokens (approximate)
fn find_name_span(name: &str, tokens: &[Token], _text: &str) -> Option<Span> {
    for token in tokens {
        if let otterc_lexer::token::TokenKind::Identifier(ref id) = token.kind
            && id == name
        {
            return Some(token.span);
        }
    }
    None
}

/// Format function signature for display
fn format_function_signature(func: &Function) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty_str = p
                .as_ref()
                .ty
                .as_ref()
                .map(|t| format!(": {}", format_type(t.as_ref())))
                .unwrap_or_default();
            format!("{}{}", p.as_ref().name, ty_str)
        })
        .collect();
    let ret_ty = func
        .ret_ty
        .as_ref()
        .map(|t| format!(" -> {}", format_type(t.as_ref())))
        .unwrap_or_default();
    format!("fn {}({}){}", func.name, params.join(", "), ret_ty)
}

fn format_callable_signature(callable: &CallableInfo) -> String {
    let params = callable
        .params
        .iter()
        .map(|p| match &p.ty {
            Some(ty) => format!("{}: {}", p.name, ty),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let ret = callable
        .return_type
        .as_ref()
        .map(|ty| format!(" -> {}", ty))
        .unwrap_or_default();
    format!("fn {}({}){}", callable.name, params, ret)
}

/// Format type for display
fn format_type(ty: &Type) -> String {
    match ty {
        Type::Simple(name) => name.clone(),
        Type::Generic { base, args } => {
            let args_str: Vec<String> = args.iter().map(|t| format_type(t.as_ref())).collect();
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
                            .map(|diag| otter_diag_to_lsp(DiagnosticKind::Type, &diag, text))
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
                    .map(|err| {
                        otter_diag_to_lsp(
                            DiagnosticKind::Parser,
                            &err.to_diagnostic(source_id),
                            text,
                        )
                    })
                    .collect();
                (diagnostics, SymbolTable::new())
            }
        },
        Err(errors) => {
            let diagnostics = errors
                .into_iter()
                .map(|err| {
                    otter_diag_to_lsp(
                        DiagnosticKind::Lexer,
                        &lexer_error_to_diag(source_id, &err),
                        text,
                    )
                })
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

#[expect(dead_code, reason = "Work in progress")]
fn collect_identifiers(text: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for token in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if token.len() > 1 && token.chars().next().is_some_and(|c| c.is_alphabetic()) {
            set.insert(token.to_string());
        }
    }
    set.into_iter().collect()
}

#[derive(Clone, Copy)]
enum DiagnosticKind {
    Lexer,
    Parser,
    Type,
}

impl DiagnosticKind {
    fn code(self) -> &'static str {
        match self {
            DiagnosticKind::Lexer => "lexer",
            DiagnosticKind::Parser => "parser",
            DiagnosticKind::Type => "typecheck",
        }
    }
}

fn lexer_error_to_diag(source: &str, err: &LexerError) -> OtterDiagnostic {
    err.to_diagnostic(source)
}

fn otter_diag_to_lsp(kind: DiagnosticKind, diag: &OtterDiagnostic, text: &str) -> Diagnostic {
    let range = span_to_range(diag.span(), text);
    let mut message = diag.message().to_string();

    if let Some(snippet) = snippet_with_highlight(text, diag.span()) {
        message.push('\n');
        message.push_str(&snippet);
    }

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
        code: Some(NumberOrString::String(kind.code().into())),
        code_description: None,
        source: Some("otterlang".into()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn snippet_with_highlight(text: &str, span: Span) -> Option<String> {
    if span.start() >= text.len() {
        return None;
    }

    let line_start = text[..span.start()]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = text[span.start()..]
        .find('\n')
        .map(|idx| span.start() + idx)
        .unwrap_or(text.len());
    let line = text[line_start..line_end].trim_end_matches(['\r']);
    let highlight_start = span.start().saturating_sub(line_start);
    let highlight_len = span
        .end()
        .saturating_sub(span.start())
        .min(line.len().saturating_sub(highlight_start));
    let mut marker = String::new();
    for _ in 0..highlight_start {
        marker.push(' ');
    }
    let carets = highlight_len.max(1);
    for _ in 0..carets {
        marker.push('^');
    }
    Some(format!("{}\n{}", line, marker))
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

fn position_to_offset(text: &str, position: Position) -> usize {
    let mut offset = 0usize;
    for (current_line, line) in text.split_inclusive('\n').enumerate() {
        if current_line == position.line as usize {
            let mut byte_index = 0usize;
            let mut seen_chars = 0usize;
            for (idx, ch) in line.char_indices() {
                if seen_chars == position.character as usize {
                    byte_index = idx;
                    break;
                }
                seen_chars += 1;
                byte_index = idx + ch.len_utf8();
            }
            let target = if seen_chars >= position.character as usize {
                byte_index
            } else {
                line.len()
            };
            return offset + target;
        }
        offset += line.len();
    }
    text.len()
}

fn find_call_context(text: &str, offset: usize) -> Option<(String, usize)> {
    if offset == 0 || offset > text.len() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut idx = offset;
    let mut depth = 0i32;
    while idx > 0 {
        idx -= 1;
        let ch = bytes[idx] as char;
        match ch {
            '(' => {
                if depth == 0 {
                    let mut name_end = idx;
                    while name_end > 0 && bytes[name_end - 1].is_ascii_whitespace() {
                        name_end -= 1;
                    }
                    if name_end == 0 {
                        return None;
                    }
                    let mut name_start = name_end;
                    while name_start > 0 {
                        let c = bytes[name_start - 1] as char;
                        if c.is_alphanumeric() || c == '_' || c == '.' {
                            name_start -= 1;
                        } else {
                            break;
                        }
                    }
                    let func_segment = text[name_start..name_end].trim();
                    if func_segment.is_empty() {
                        return None;
                    }
                    let func_name = func_segment.rsplit('.').next()?.to_string();
                    let args_slice = &text[idx + 1..offset];
                    let mut param_depth = 0i32;
                    let mut commas = 0usize;
                    for ch in args_slice.chars() {
                        match ch {
                            '(' | '[' | '{' => param_depth += 1,
                            ')' | ']' | '}' => {
                                if param_depth > 0 {
                                    param_depth -= 1;
                                }
                            }
                            ',' if param_depth == 0 => commas += 1,
                            _ => {}
                        }
                    }
                    return Some((func_name, commas));
                } else {
                    depth -= 1;
                }
            }
            ')' => depth += 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::print_stdout,
        reason = "Printing to stdout is acceptable in tests"
    )]
    #![expect(clippy::panic, reason = "Panicking on test failures is acceptable")]

    use super::*;

    #[test]
    fn test_build_symbol_table() {
        let test_code = r#"
let x = 10
let y = 20

fn add(a, b):
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
