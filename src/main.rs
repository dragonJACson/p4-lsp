use dashmap::DashMap;
use log::info;
use ropey::Rope;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use p4::ast::AST;
use p4::check::{self, Diagnostics, Level};
use p4::error::{Error, PreprocessorError};
use p4::lexer::Lexer;
use p4::parser::Parser;
use tower_lsp::lsp_types::SemanticTokenType;

mod analysis;

#[derive(Debug)]
struct Document {
    uri: Url,
    content: Rope,
    ast: Option<AST>,
    diagnostics: Vec<Diagnostic>,
    macros: Vec<(String, String)>,
}

struct Backend {
    client: Client,
    documents: DashMap<Url, Document>,
    semantic_tokens: SemanticTokensLegend,
}

struct SemanticTokensBuilder {
    data: Vec<SemanticToken>,
    token_types: Vec<SemanticTokenType>,
    token_modifiers: Vec<SemanticTokenModifier>,
}

impl SemanticTokensBuilder {
    fn new(legend: &SemanticTokensLegend) -> Self {
        Self {
            data: Vec::new(),
            token_types: legend.token_types.clone(),
            token_modifiers: legend.token_modifiers.clone(),
        }
    }

    fn push(
        &mut self,
        line: u32,
        start: u32,
        length: u32,
        token_type: SemanticTokenType,
        modifiers: &[SemanticTokenModifier],
    ) {
        let Some(token_type_index) = self
            .token_types
            .iter()
            .position(|value| value == &token_type)
            .map(|index| index as u32)
        else {
            return;
        };

        let mut modifier_bitset = 0u32;
        for modifier in modifiers {
            if let Some(index) = self
                .token_modifiers
                .iter()
                .position(|value| value == modifier)
            {
                modifier_bitset |= 1 << index;
            }
        }

        self.data.push(SemanticToken {
            delta_line: line,
            delta_start: start,
            length,
            token_type: token_type_index,
            token_modifiers_bitset: modifier_bitset,
        });
    }

    fn build(self) -> SemanticTokens {
        SemanticTokens {
            result_id: None,
            data: self.data,
        }
    }
}

impl Backend {
    fn new(client: Client) -> Self {
        let semantic_tokens = SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::KEYWORD,
                SemanticTokenType::TYPE,
                SemanticTokenType::FUNCTION,
                SemanticTokenType::METHOD,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::PARAMETER,
                SemanticTokenType::MACRO,
                SemanticTokenType::NUMBER,
                SemanticTokenType::STRING,
                SemanticTokenType::OPERATOR,
            ],
            token_modifiers: Vec::new(),
        };

        Self {
            client,
            documents: DashMap::new(),
            semantic_tokens,
        }
    }

    fn parse_document(&self, uri: &Url, text: &str) -> Document {
        let rope = Rope::from_str(text);
        let filename = Arc::new(uri.path().to_string());
        let mut lsp_diagnostics = Vec::new();

        let lines: Vec<&str> = text.lines().collect();

        let (ast, diags, macros) = match self.compile_p4(text, filename.clone()) {
            Ok((ast, diags, macros)) => (Some(ast), diags, macros),
            Err(err) => {
                match err {
                    CompileError::Preprocessor(e) => {
                        lsp_diagnostics.push(Diagnostic {
                            range: Range {
                                start: Position {
                                    line: e.line as u32,
                                    character: 0,
                                },
                                end: Position {
                                    line: e.line as u32,
                                    character: lines.get(e.line).map(|l| l.len() as u32).unwrap_or(0),
                                },
                            },
                            severity: Some(DiagnosticSeverity::ERROR),
                            message: e.message,
                            ..Default::default()
                        });
                    }
                    CompileError::P4(e) => {
                        self.convert_p4_error(&e, &lines, &mut lsp_diagnostics);
                    }
                }
                (None, Diagnostics::new(), Vec::new())
            }
        };

        for diag in &diags.0 {
            let severity = match diag.level {
                Level::Error => DiagnosticSeverity::ERROR,
                Level::Warning => DiagnosticSeverity::WARNING,
                Level::Deprecation => DiagnosticSeverity::WARNING,
                Level::Info => DiagnosticSeverity::INFORMATION,
            };

            let line = diag.token.line as u32;
            let col = diag.token.col as u32;

            lsp_diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line,
                        character: col,
                    },
                    end: Position {
                        line,
                        character: col + 1,
                    },
                },
                severity: Some(severity),
                message: diag.message.clone(),
                ..Default::default()
            });
        }

        Document {
            uri: uri.clone(),
            content: rope,
            ast,
            diagnostics: lsp_diagnostics,
            macros,
        }
    }

    fn compile_p4(
        &self,
        text: &str,
        filename: Arc<String>,
    ) -> std::result::Result<(AST, Diagnostics, Vec<(String, String)>), CompileError> {
        let mut ast = AST::default();
        let mut macros = Vec::new();
        self.process_file_content(text, filename, &mut ast, &mut macros)?;

        let (_, diags) = check::all(&ast);
        Ok((ast, diags, macros))
    }

    fn process_file_content(
        &self,
        text: &str,
        filename: Arc<String>,
        ast: &mut AST,
        macros: &mut Vec<(String, String)>,
    ) -> std::result::Result<(), CompileError> {
        let ppr = p4::preprocessor::run(text, filename.clone())
            .map_err(CompileError::Preprocessor)?;

        for m in &ppr.elements.macros {
            macros.push((m.name.clone(), m.body.clone()));
        }

        for included in &ppr.elements.includes {
            let path = Path::new(included);
            let include_path = if !path.is_absolute() {
                let parent = Path::new(&*filename).parent().unwrap_or(Path::new("."));
                parent.join(included)
            } else {
                path.to_path_buf()
            };

            if let Ok(include_content) = fs::read_to_string(&include_path) {
                let include_filename = Arc::new(include_path.to_string_lossy().to_string());
                self.process_file_content(&include_content, include_filename, ast, macros)?;
            }
        }

        let lexer = Lexer::new(ppr.lines.iter().map(|s| s.as_str()).collect(), filename);
        let mut parser = Parser::new(lexer);
        parser.run(ast).map_err(CompileError::P4)?;

        Ok(())
    }

    fn convert_p4_error(&self, error: &Error, lines: &[&str], lsp_diagnostics: &mut Vec<Diagnostic>) {
        match error {
            Error::Lexer(e) => {
                lsp_diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: e.line as u32,
                            character: e.col as u32,
                        },
                        end: Position {
                            line: e.line as u32,
                            character: (e.col + e.len) as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Lexer error: {}", e),
                    ..Default::default()
                });
            }
            Error::Parser(e) => {
                lsp_diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: e.at.line as u32,
                            character: e.at.col as u32,
                        },
                        end: Position {
                            line: e.at.line as u32,
                            character: lines.get(e.at.line).map(|l| l.len() as u32).unwrap_or(e.at.col as u32 + 1),
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: e.message.clone(),
                    ..Default::default()
                });
            }
            Error::Semantic(errors) => {
                for e in errors {
                    lsp_diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position {
                                line: e.at.line as u32,
                                character: e.at.col as u32,
                            },
                            end: Position {
                                line: e.at.line as u32,
                                character: lines.get(e.at.line).map(|l| l.len() as u32).unwrap_or(e.at.col as u32 + 1),
                            },
                        },
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: e.message.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    fn get_macro_hover(&self, doc: &Document, position: Position) -> Option<String> {
        if doc.macros.is_empty() {
            return None;
        }
        let line_idx = position.line as usize;
        let col_idx = position.character as usize;

        if line_idx >= doc.content.len_lines() {
            return None;
        }

        let line = doc.content.line(line_idx);
        let line_str: String = line.chars().collect();

        let (word, _) = analysis::extract_word_and_prefix(&line_str, col_idx);
        let word = word?;

        for (name, body) in &doc.macros {
            if name == &word {
                return Some(format!("```p4\n{}\n```", body));
            }
        }

        None
    }

    fn get_include_location(
        &self,
        doc: &Document,
        position: Position,
    ) -> Option<Location> {
        let line_idx = position.line as usize;
        if line_idx >= doc.content.len_lines() {
            return None;
        }

        let line = doc.content.line(line_idx);
        let line_str: String = line.chars().collect();

        let trimmed = line_str.trim_start();
        if !trimmed.starts_with("#include") {
            return None;
        }

        let include_start = trimmed.find("#include")? + "#include".len();
        let include_section = &trimmed[include_start..].trim();
        let (path, start_offset) = if include_section.starts_with('<') {
            let end = include_section.find('>')?;
            (include_section[1..end].to_string(), 1)
        } else if include_section.starts_with('"') {
            let end = include_section[1..].find('"')? + 1;
            (include_section[1..end].to_string(), 1)
        } else {
            return None;
        };

        let filename = doc.uri.path();
        let include_path = resolve_include_path(filename, &path)?;
        let include_uri = Url::from_file_path(&include_path).ok()?;
        let col = trimmed.find(&path)? + start_offset;

        Some(Location {
            uri: include_uri,
            range: Range {
                start: Position {
                    line: line_idx as u32,
                    character: col as u32,
                },
                end: Position {
                    line: line_idx as u32,
                    character: (col + path.len()) as u32,
                },
            },
        })
    }
}

fn resolve_include_path(current_file: &str, include: &str) -> Option<String> {
    let include_path = Path::new(include);
    if include_path.is_absolute() {
        return Some(include_path.to_string_lossy().to_string());
    }
    let parent = Path::new(current_file).parent().unwrap_or(Path::new("."));
    Some(parent.join(include).to_string_lossy().to_string())
}



enum CompileError {
    Preprocessor(PreprocessorError),
    P4(Error),
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
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                    SemanticTokensOptions {
                        legend: self.semantic_tokens.clone(),
                        range: Some(true),
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "p4-lsp".to_string(),
                version: Some("0.1.0".to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("P4 Language Server initialized");
        self.client
            .log_message(MessageType::INFO, "P4 Language Server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = &params.text_document.text;
        
        let doc = self.parse_document(&uri, text);
        let diagnostics = doc.diagnostics.clone();
        self.documents.insert(uri.clone(), doc);

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().next() {
            let doc = self.parse_document(&uri, &change.text);
            let diagnostics = doc.diagnostics.clone();
            self.documents.insert(uri.clone(), doc);

            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        if let Some(doc) = self.documents.get(uri) {
            if let Some(ast) = &doc.ast {
                if let Some(hover_info) = analysis::get_hover_info(ast, &doc.content, position) {
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: hover_info,
                        }),
                        range: None,
                    }));
                }
            }

            if let Some(macro_hover) = self.get_macro_hover(&doc, position) {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: macro_hover,
                    }),
                    range: None,
                }));
            }
        }
        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        if let Some(doc) = self.documents.get(uri) {
            if let Some(ast) = &doc.ast {
                let completions = analysis::get_completions_with_context(ast, &doc.content, position);
                return Ok(Some(CompletionResponse::Array(completions)));
            }
        }
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        if let Some(doc) = self.documents.get(uri) {
            if let Some(location) = self.get_include_location(&doc, position) {
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            }

            if let Some(ast) = &doc.ast {
                if let Some(location) = analysis::get_definition_location(ast, &doc.content, position) {
                    return Ok(Some(GotoDefinitionResponse::Scalar(location)));
                }
            }
        }

        Ok(None)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let tokens = self.collect_semantic_tokens(&doc);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        })))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let uri = &params.text_document.uri;
        let doc = match self.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let tokens = self.collect_semantic_tokens_range(&doc, params.range);
        Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        })))
    }
}

impl Backend {
    fn collect_semantic_tokens(&self, doc: &Document) -> Vec<SemanticToken> {
        let mut builder = SemanticTokensBuilder::new(&self.semantic_tokens);
        self.add_lexical_semantic_tokens(doc, &mut builder);

        if let Some(ast) = &doc.ast {
            let mut add = |token: &p4::lexer::Token, token_type: SemanticTokenType| {
                if let Some(relative) = token_relative(token, &doc.content) {
                    builder.push(
                        relative.0,
                        relative.1,
                        relative.2,
                        token_type,
                        &[],
                    );
                }
            };

            for header in &ast.headers {
                add(&header.token, SemanticTokenType::TYPE);
                for m in &header.members {
                    add(&m.token, SemanticTokenType::VARIABLE);
                }
            }
            for s in &ast.structs {
                add(&s.token, SemanticTokenType::TYPE);
                for m in &s.members {
                    add(&m.token, SemanticTokenType::VARIABLE);
                }
            }
            for td in &ast.typedefs {
                add(&td.token, SemanticTokenType::TYPE);
            }
            for c in &ast.constants {
                add(&c.token, SemanticTokenType::VARIABLE);
            }
            for pkg in &ast.packages {
                add(&pkg.token, SemanticTokenType::TYPE);
                for p in &pkg.parameters {
                    add(&p.token, SemanticTokenType::PARAMETER);
                }
            }
            for control in &ast.controls {
                add(&control.token, SemanticTokenType::TYPE);
                for p in &control.parameters {
                    add(&p.name_token, SemanticTokenType::PARAMETER);
                }
                for v in &control.variables {
                    add(&v.token, SemanticTokenType::VARIABLE);
                }
                for a in &control.actions {
                    add(&a.token, SemanticTokenType::FUNCTION);
                    for p in &a.parameters {
                        add(&p.name_token, SemanticTokenType::PARAMETER);
                    }
                }
                for t in &control.tables {
                    add(&t.token, SemanticTokenType::VARIABLE);
                }
            }
            for parser in &ast.parsers {
                add(&parser.token, SemanticTokenType::TYPE);
                for p in &parser.parameters {
                    add(&p.name_token, SemanticTokenType::PARAMETER);
                }
                for s in &parser.states {
                    add(&s.token, SemanticTokenType::FUNCTION);
                }
            }
            for ext in &ast.externs {
                add(&ext.token, SemanticTokenType::TYPE);
                for m in &ext.methods {
                    add(&m.token, SemanticTokenType::METHOD);
                    for p in &m.parameters {
                        add(&p.name_token, SemanticTokenType::PARAMETER);
                    }
                }
            }
        }

        for (name, _body) in &doc.macros {
            for (line_idx, line) in doc.content.lines().enumerate() {
                let line_str: String = line.chars().collect();
                if let Some(col) = line_str.find(name) {
                    builder.push(
                        line_idx as u32,
                        col as u32,
                        name.len() as u32,
                        SemanticTokenType::MACRO,
                        &[],
                    );
                }
            }
        }

        encode_relative_tokens(builder.build().data)
    }

    fn collect_semantic_tokens_range(
        &self,
        doc: &Document,
        range: Range,
    ) -> Vec<SemanticToken> {
        let all = self.collect_semantic_tokens(doc);
        filter_tokens_by_range(all, range)
    }

    fn add_lexical_semantic_tokens(
        &self,
        doc: &Document,
        builder: &mut SemanticTokensBuilder,
    ) {
        let lines: Vec<String> = doc
            .content
            .lines()
            .map(|line| line.chars().collect())
            .collect();

        let mut lexer = Lexer::new(
            lines.iter().map(|s| s.as_str()).collect(),
            Arc::new(String::from("<memory>")),
        );
        loop {
            let token = match lexer.next() {
                Ok(token) => token,
                Err(_) => break,
            };

            let (token_type, length) = match &token.kind {
                p4::lexer::Kind::Const
                | p4::lexer::Kind::Header
                | p4::lexer::Kind::Typedef
                | p4::lexer::Kind::Control
                | p4::lexer::Kind::Struct
                | p4::lexer::Kind::Action
                | p4::lexer::Kind::Parser
                | p4::lexer::Kind::Table
                | p4::lexer::Kind::Size
                | p4::lexer::Kind::Key
                | p4::lexer::Kind::Exact
                | p4::lexer::Kind::Ternary
                | p4::lexer::Kind::Lpm
                | p4::lexer::Kind::Range
                | p4::lexer::Kind::Actions
                | p4::lexer::Kind::DefaultAction
                | p4::lexer::Kind::Entries
                | p4::lexer::Kind::In
                | p4::lexer::Kind::InOut
                | p4::lexer::Kind::Out
                | p4::lexer::Kind::Transition
                | p4::lexer::Kind::State
                | p4::lexer::Kind::Select
                | p4::lexer::Kind::Apply
                | p4::lexer::Kind::Package
                | p4::lexer::Kind::Extern
                | p4::lexer::Kind::If
                | p4::lexer::Kind::Else
                | p4::lexer::Kind::Return => (Some(SemanticTokenType::KEYWORD), 1),
                p4::lexer::Kind::Bool
                | p4::lexer::Kind::Error
                | p4::lexer::Kind::Bit
                | p4::lexer::Kind::Varbit
                | p4::lexer::Kind::Int
                | p4::lexer::Kind::String => (Some(SemanticTokenType::TYPE), 1),
                p4::lexer::Kind::PoundDefine | p4::lexer::Kind::PoundInclude => {
                    (Some(SemanticTokenType::MACRO), 1)
                }
                p4::lexer::Kind::IntLiteral(value) => {
                    (Some(SemanticTokenType::NUMBER), value.to_string().len() as u32)
                }
                p4::lexer::Kind::BitLiteral(width, value) => {
                    let text = format!("{}w{}", width, value);
                    (Some(SemanticTokenType::NUMBER), text.len() as u32)
                }
                p4::lexer::Kind::SignedLiteral(width, value) => {
                    let text = format!("{}s{}", width, value);
                    (Some(SemanticTokenType::NUMBER), text.len() as u32)
                }
                p4::lexer::Kind::TrueLiteral => (Some(SemanticTokenType::NUMBER), 4),
                p4::lexer::Kind::FalseLiteral => (Some(SemanticTokenType::NUMBER), 5),
                p4::lexer::Kind::StringLiteral(value) => {
                    (Some(SemanticTokenType::STRING), (value.len() + 2) as u32)
                }
                p4::lexer::Kind::DoubleEquals => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::NotEquals => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::Equals => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Plus => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Minus => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Mod => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Dot => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Mask => (Some(SemanticTokenType::OPERATOR), 3),
                p4::lexer::Kind::LogicalAnd => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::And => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Bang => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Tilde => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Shl => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::Pipe => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::Carat => (Some(SemanticTokenType::OPERATOR), 1),
                p4::lexer::Kind::GreaterThanEquals => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::LessThanEquals => (Some(SemanticTokenType::OPERATOR), 2),
                p4::lexer::Kind::Identifier(value) => {
                    (Some(SemanticTokenType::VARIABLE), value.len() as u32)
                }
                _ => (None, 1),
            };

            if let Some(token_type) = token_type {
                builder.push(
                    token.line as u32,
                    token.col as u32,
                    length,
                    token_type,
                    &[],
                );
            }

            if token.kind == p4::lexer::Kind::Eof {
                break;
            }
        }
    }
}

fn token_relative(token: &p4::lexer::Token, content: &Rope) -> Option<(u32, u32, u32)> {
    let line = token.line;
    let col = token.col;
    if line >= content.len_lines() {
        return None;
    }

    let length = match &token.kind {
        p4::lexer::Kind::Identifier(value) => value.len(),
        p4::lexer::Kind::IntLiteral(value) => value.to_string().len(),
        p4::lexer::Kind::BitLiteral(width, value) => format!("{}w{}", width, value).len(),
        p4::lexer::Kind::SignedLiteral(width, value) => {
            format!("{}s{}", width, value).len()
        }
        p4::lexer::Kind::StringLiteral(value) => value.len() + 2,
        p4::lexer::Kind::TrueLiteral => 4,
        p4::lexer::Kind::FalseLiteral => 5,
        p4::lexer::Kind::Const => 5,
        p4::lexer::Kind::Header => 6,
        p4::lexer::Kind::Typedef => 7,
        p4::lexer::Kind::Control => 7,
        p4::lexer::Kind::Struct => 6,
        p4::lexer::Kind::Action => 6,
        p4::lexer::Kind::Parser => 6,
        p4::lexer::Kind::Table => 5,
        p4::lexer::Kind::Size => 4,
        p4::lexer::Kind::Key => 3,
        p4::lexer::Kind::Exact => 5,
        p4::lexer::Kind::Ternary => 7,
        p4::lexer::Kind::Lpm => 3,
        p4::lexer::Kind::Range => 5,
        p4::lexer::Kind::Actions => 7,
        p4::lexer::Kind::DefaultAction => 14,
        p4::lexer::Kind::Entries => 7,
        p4::lexer::Kind::In => 2,
        p4::lexer::Kind::InOut => 5,
        p4::lexer::Kind::Out => 3,
        p4::lexer::Kind::Transition => 10,
        p4::lexer::Kind::State => 5,
        p4::lexer::Kind::Select => 6,
        p4::lexer::Kind::Apply => 5,
        p4::lexer::Kind::Package => 7,
        p4::lexer::Kind::Extern => 6,
        p4::lexer::Kind::If => 2,
        p4::lexer::Kind::Else => 4,
        p4::lexer::Kind::Return => 6,
        p4::lexer::Kind::Bool => 4,
        p4::lexer::Kind::Error => 5,
        p4::lexer::Kind::Bit => 3,
        p4::lexer::Kind::Varbit => 6,
        p4::lexer::Kind::Int => 3,
        p4::lexer::Kind::String => 6,
        p4::lexer::Kind::PoundDefine => 7,
        p4::lexer::Kind::PoundInclude => 8,
        p4::lexer::Kind::DoubleEquals => 2,
        p4::lexer::Kind::NotEquals => 2,
        p4::lexer::Kind::Equals => 1,
        p4::lexer::Kind::Plus => 1,
        p4::lexer::Kind::Minus => 1,
        p4::lexer::Kind::Mod => 1,
        p4::lexer::Kind::Dot => 1,
        p4::lexer::Kind::Mask => 3,
        p4::lexer::Kind::LogicalAnd => 2,
        p4::lexer::Kind::And => 1,
        p4::lexer::Kind::Bang => 1,
        p4::lexer::Kind::Tilde => 1,
        p4::lexer::Kind::Shl => 2,
        p4::lexer::Kind::Pipe => 1,
        p4::lexer::Kind::Carat => 1,
        p4::lexer::Kind::GreaterThanEquals => 2,
        p4::lexer::Kind::LessThanEquals => 2,
        _ => 1,
    };

    Some((line as u32, col as u32, length as u32))
}

fn filter_tokens_by_range(tokens: Vec<SemanticToken>, range: Range) -> Vec<SemanticToken> {
    let filtered: Vec<SemanticToken> = tokens
        .into_iter()
        .filter(|t| {
            let line = t.delta_line as u32;
            let col = t.delta_start as u32;
            if line < range.start.line || line > range.end.line {
                return false;
            }
            if line == range.start.line && col < range.start.character {
                return false;
            }
            if line == range.end.line && col > range.end.character {
                return false;
            }
            true
        })
        .collect();
    encode_relative_tokens(filtered)
}

fn encode_relative_tokens(tokens: Vec<SemanticToken>) -> Vec<SemanticToken> {
    let mut sorted = tokens;
    sorted.sort_by_key(|t| (t.delta_line, t.delta_start));
    let mut last_line: u32 = 0;
    let mut last_start: u32 = 0;
    let mut encoded = Vec::new();

    for mut token in sorted {
        let current_line = token.delta_line;
        let current_start = token.delta_start;
        let delta_line = current_line.saturating_sub(last_line);
        let delta_start = if delta_line == 0 {
            current_start.saturating_sub(last_start)
        } else {
            current_start
        };

        token.delta_line = delta_line;
        token.delta_start = delta_start;
        encoded.push(token);

        last_line = current_line;
        last_start = current_start;
    }

    encoded

}


#[tokio::main]
async fn main() {
    env_logger::init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
