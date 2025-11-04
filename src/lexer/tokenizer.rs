use super::token::{Span, Token, TokenKind};
use crate::utils::errors::{Diagnostic, DiagnosticSeverity};
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum LexerError {
    #[error("tabs are not allowed for indentation (line {line}, column {column})")]
    TabsNotAllowed {
        line: usize,
        column: usize,
        span: Span,
    },
    #[error("indentation mismatch: expected {expected} spaces, found {found} (line {line})")]
    IndentationMismatch {
        line: usize,
        expected: usize,
        found: usize,
        span: Span,
    },
    #[error("unterminated string literal (line {line}, column {column})")]
    UnterminatedString {
        line: usize,
        column: usize,
        span: Span,
    },
    #[error("unexpected character `{ch}` (line {line}, column {column})")]
    UnexpectedCharacter {
        ch: char,
        line: usize,
        column: usize,
        span: Span,
    },
}

impl LexerError {
    pub fn to_diagnostic(&self, source_id: &str) -> Diagnostic {
        match self {
            LexerError::TabsNotAllowed { span, .. } => Diagnostic::new(
                DiagnosticSeverity::Error,
                source_id,
                span.clone(),
                self.to_string(),
            )
            .with_suggestion("Use spaces instead of tabs for indentation")
            .with_help(
                "OtterLang uses spaces for indentation. Configure your editor to use spaces.",
            ),
            LexerError::IndentationMismatch {
                span,
                expected,
                found,
                ..
            } => Diagnostic::new(
                DiagnosticSeverity::Error,
                source_id,
                span.clone(),
                self.to_string(),
            )
            .with_suggestion(format!("Indent with {} spaces (found {})", expected, found))
            .with_help("Check that indentation is consistent throughout the file."),
            LexerError::UnterminatedString { span, .. } => Diagnostic::new(
                DiagnosticSeverity::Error,
                source_id,
                span.clone(),
                self.to_string(),
            )
            .with_suggestion("Add a closing quote (\") to terminate the string")
            .with_help("String literals must be enclosed in double quotes."),
            LexerError::UnexpectedCharacter { span, ch, .. } => {
                let mut diag = Diagnostic::new(
                    DiagnosticSeverity::Error,
                    source_id,
                    span.clone(),
                    self.to_string(),
                );

                // Provide suggestions for common typos
                match ch {
                    '`' => {
                        diag = diag.with_suggestion(
                            "Did you mean a single quote (') or double quote (\")?",
                        )
                    }
                    '~' => diag = diag.with_suggestion("Did you mean tilde (~) or negation (not)?"),
                    '@' => {
                        diag = diag
                            .with_suggestion("Did you mean the at symbol (@) or member access (.)?")
                    }
                    _ => {
                        if ch.is_ascii_punctuation() {
                            diag = diag.with_suggestion("Check for typos or invalid characters");
                        }
                    }
                }

                diag.with_help("This character is not valid in OtterLang syntax.")
            }
        }
    }
}

pub type LexResult<T> = Result<T, Vec<LexerError>>;

// Optimized lexer state machine
struct LexerState {
    tokens: Vec<Token>,
    errors: Vec<LexerError>,
    indent_stack: Vec<usize>,
    source: Vec<u8>,
    offset: usize,
    line: usize,
    column: usize,
}

impl LexerState {
    fn new(source: &str) -> Self {
        Self {
            tokens: Vec::new(),
            errors: Vec::new(),
            indent_stack: vec![0],
            source: source.as_bytes().to_vec(),
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    fn current_char(&self) -> Option<u8> {
        self.source.get(self.offset).copied()
    }

    fn peek_char(&self, ahead: usize) -> Option<u8> {
        self.source.get(self.offset + ahead).copied()
    }

    fn advance(&mut self, count: usize) {
        for _ in 0..count {
            if self.current_char() == Some(b'\n') {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            self.offset += 1;
        }
    }

    fn create_span(&self, start: usize, len: usize) -> Span {
        Span::new(start, start + len)
    }

    fn emit_token(&mut self, kind: TokenKind, start: usize, len: usize) {
        let span = Span::new(start, start + len);
        self.tokens.push(Token::new(kind, span));
    }

    fn emit_error(&mut self, error: LexerError) {
        self.errors.push(error);
    }

    fn is_at_end(&self) -> bool {
        self.offset >= self.source.len()
    }
}

pub fn tokenize(source: &str) -> LexResult<Vec<Token>> {
    let mut state = LexerState::new(source);

    // Pre-allocate capacity for better performance
    let estimated_tokens = source.len() / 4; // Rough estimate
    state.tokens.reserve(estimated_tokens);

    while !state.is_at_end() {
        state.process_line();
    }

    // Finalize indentation and add EOF
    state.finalize_indentation();

    if state.errors.is_empty() {
        Ok(state.tokens)
    } else {
        Err(state.errors)
    }
}

impl LexerState {
    fn process_line(&mut self) {
        let line_start = self.offset;
        let mut indent_width = 0;

        // Process indentation
        while let Some(ch) = self.current_char() {
            match ch {
                b' ' => {
                    indent_width += 1;
                    self.advance(1);
                }
                b'\t' => {
                    let span = self.create_span(self.offset, 1);
                    self.emit_error(LexerError::TabsNotAllowed {
                        line: self.line,
                        column: self.column,
                        span,
                    });
                    self.advance(1);
                }
                b'\n' => {
                    // Empty line, just add newline
                    self.emit_token(TokenKind::Newline, self.offset, 1);
                    self.advance(1);
                    return;
                }
                b'#' => {
                    // Comment line, skip to end
                    self.skip_to_end_of_line();
                    return;
                }
                _ => break,
            }
        }

        let rest_start = self.offset;
        let is_blank = self.skip_whitespace_and_check_blank();

        if is_blank {
            // Add newline if we haven't already
            if let Some(b'\n') = self.current_char() {
                self.emit_token(TokenKind::Newline, self.offset, 1);
                self.advance(1);
            }
            return;
        }

        // Handle indentation changes
        self.handle_indentation(indent_width, line_start);

        // Tokenize the rest of the line
        self.tokenize_line_content(rest_start);
    }

    fn skip_whitespace_and_check_blank(&mut self) -> bool {
        let mut has_non_whitespace = false;
        while let Some(ch) = self.current_char() {
            match ch {
                b' ' | b'\t' => {
                    self.advance(1);
                }
                b'\n' => break,
                b'#' => break,
                _ => {
                    has_non_whitespace = true;
                    break;
                }
            }
        }
        !has_non_whitespace
    }

    fn handle_indentation(&mut self, current_indent: usize, line_start: usize) {
        let last_indent = *self.indent_stack.last().unwrap();

        if current_indent > last_indent {
            self.indent_stack.push(current_indent);
            self.emit_token(
                TokenKind::Indent,
                line_start + last_indent,
                current_indent - last_indent,
            );
        } else if current_indent < last_indent {
            while current_indent < *self.indent_stack.last().unwrap() {
                let top = self.indent_stack.pop().unwrap();
                self.emit_token(
                    TokenKind::Dedent,
                    line_start + current_indent,
                    top - current_indent,
                );
            }
            if current_indent != *self.indent_stack.last().unwrap() {
                let span = self.create_span(line_start + current_indent, 1);
                self.emit_error(LexerError::IndentationMismatch {
                    line: self.line,
                    expected: *self.indent_stack.last().unwrap(),
                    found: current_indent,
                    span,
                });
            }
        }
    }

    fn tokenize_line_content(&mut self, start: usize) {
        while !self.is_at_end() {
            let ch = match self.current_char() {
                Some(ch) => ch,
                None => break,
            };

            match ch {
                b'\n' => {
                    self.emit_token(TokenKind::Newline, self.offset, 1);
                    self.advance(1);
                    return;
                }
                b'#' => {
                    self.skip_to_end_of_line();
                    return;
                }
                b' ' | b'\t' => {
                    self.advance(1);
                }
                _ => {
                    self.tokenize_token(start);
                }
            }
        }
    }

    fn tokenize_token(&mut self, _line_start: usize) {
        let _token_start = self.offset;

        match self.current_char().unwrap() {
            b'(' => {
                self.emit_token(TokenKind::LParen, self.offset, 1);
                self.advance(1);
            }
            b')' => {
                self.emit_token(TokenKind::RParen, self.offset, 1);
                self.advance(1);
            }
            b'{' => {
                self.emit_token(TokenKind::LBrace, self.offset, 1);
                self.advance(1);
            }
            b'}' => {
                self.emit_token(TokenKind::RBrace, self.offset, 1);
                self.advance(1);
            }
            b'[' => {
                self.emit_token(TokenKind::LBracket, self.offset, 1);
                self.advance(1);
            }
            b']' => {
                self.emit_token(TokenKind::RBracket, self.offset, 1);
                self.advance(1);
            }
            b',' => {
                self.emit_token(TokenKind::Comma, self.offset, 1);
                self.advance(1);
            }
            b'.' => {
                if self.peek_char(1) == Some(b'.') {
                    self.emit_token(TokenKind::DoubleDot, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Dot, self.offset, 1);
                    self.advance(1);
                }
            }
            b':' => {
                self.emit_token(TokenKind::Colon, self.offset, 1);
                self.advance(1);
            }
            b'+' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::PlusEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Plus, self.offset, 1);
                    self.advance(1);
                }
            }
            b'-' => match self.peek_char(1) {
                Some(b'=') => {
                    self.emit_token(TokenKind::MinusEq, self.offset, 2);
                    self.advance(2);
                }
                Some(b'>') => {
                    self.emit_token(TokenKind::Arrow, self.offset, 2);
                    self.advance(2);
                }
                _ => {
                    self.emit_token(TokenKind::Minus, self.offset, 1);
                    self.advance(1);
                }
            },
            b'*' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::StarEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Star, self.offset, 1);
                    self.advance(1);
                }
            }
            b'/' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::SlashEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Slash, self.offset, 1);
                    self.advance(1);
                }
            }
            b'%' => {
                self.emit_token(TokenKind::Percent, self.offset, 1);
                self.advance(1);
            }
            b'|' => {
                self.emit_token(TokenKind::Pipe, self.offset, 1);
                self.advance(1);
            }
            b'&' => {
                self.emit_token(TokenKind::Amp, self.offset, 1);
                self.advance(1);
            }
            b'!' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::Neq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Bang, self.offset, 1);
                    self.advance(1);
                }
            }
            b'=' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::EqEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Equals, self.offset, 1);
                    self.advance(1);
                }
            }
            b'<' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::LtEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Lt, self.offset, 1);
                    self.advance(1);
                }
            }
            b'>' => {
                if self.peek_char(1) == Some(b'=') {
                    self.emit_token(TokenKind::GtEq, self.offset, 2);
                    self.advance(2);
                } else {
                    self.emit_token(TokenKind::Gt, self.offset, 1);
                    self.advance(1);
                }
            }
            b'"' => {
                self.tokenize_string();
            }
            b'f' => {
                // Check for f-string before treating as regular identifier
                if self.peek_char(1) == Some(b'"') {
                    self.tokenize_fstring();
                } else {
                    self.tokenize_identifier_or_keyword();
                }
            }
            ch if ch.is_ascii_digit() => {
                self.tokenize_number();
            }
            ch if ch.is_ascii_alphabetic() || ch == b'_' => {
                self.tokenize_identifier_or_keyword();
            }
            ch if ch > 127 => {
                self.tokenize_unicode_identifier();
            }
            _ => {
                let ch = self.current_char().unwrap();
                let span = self.create_span(self.offset, 1);
                self.emit_error(LexerError::UnexpectedCharacter {
                    ch: ch as char,
                    line: self.line,
                    column: self.column,
                    span,
                });
                self.advance(1);
            }
        }
    }

    fn tokenize_string(&mut self) {
        let start = self.offset;
        self.advance(1); // Skip opening quote

        while let Some(ch) = self.current_char() {
            match ch {
                b'"' => {
                    let value = unsafe {
                        std::str::from_utf8_unchecked(&self.source[start + 1..self.offset])
                    };
                    let span = Span::new(start, self.offset + 1);
                    self.tokens.push(Token::new(
                        TokenKind::StringLiteral(value.to_string()),
                        span,
                    ));
                    self.advance(1);
                    return;
                }
                b'\n' => {
                    let span = self.create_span(start, self.offset - start);
                    self.emit_error(LexerError::UnterminatedString {
                        line: self.line,
                        column: self.column,
                        span,
                    });
                    return;
                }
                _ => {
                    self.advance(1);
                }
            }
        }

        // Unterminated string at EOF
        let span = self.create_span(start, self.offset - start);
        self.emit_error(LexerError::UnterminatedString {
            line: self.line,
            column: self.column,
            span,
        });
    }

    fn tokenize_fstring(&mut self) {
        let start = self.offset;
        self.advance(2); // Skip f"

        while let Some(ch) = self.current_char() {
            match ch {
                b'"' => {
                    let value = unsafe {
                        std::str::from_utf8_unchecked(&self.source[start + 2..self.offset])
                    };
                    let span = Span::new(start, self.offset + 1);
                    self.tokens
                        .push(Token::new(TokenKind::FString(value.to_string()), span));
                    self.advance(1);
                    return;
                }
                b'\n' => {
                    let span = self.create_span(start, self.offset - start);
                    self.emit_error(LexerError::UnterminatedString {
                        line: self.line,
                        column: self.column,
                        span,
                    });
                    return;
                }
                _ => {
                    self.advance(1);
                }
            }
        }

        // Unterminated fstring at EOF
        let span = self.create_span(start, self.offset - start);
        self.emit_error(LexerError::UnterminatedString {
            line: self.line,
            column: self.column,
            span,
        });
    }

    fn tokenize_number(&mut self) {
        let start = self.offset;

        // Parse integer part
        while let Some(ch) = self.current_char() {
            if ch.is_ascii_digit() || ch == b'_' {
                self.advance(1);
            } else {
                break;
            }
        }

        // Parse decimal part
        if let Some(b'.') = self.current_char() {
            if let Some(next) = self.peek_char(1) {
                if next.is_ascii_digit() {
                    self.advance(1); // Skip decimal point
                    while let Some(ch) = self.current_char() {
                        if ch.is_ascii_digit() || ch == b'_' {
                            self.advance(1);
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        let value = unsafe { std::str::from_utf8_unchecked(&self.source[start..self.offset]) };
        self.emit_token(
            TokenKind::Number(value.to_string()),
            start,
            self.offset - start,
        );
    }

    fn tokenize_identifier_or_keyword(&mut self) {
        let start = self.offset;

        while let Some(ch) = self.current_char() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.advance(1);
            } else {
                break;
            }
        }

        let value = unsafe { std::str::from_utf8_unchecked(&self.source[start..self.offset]) };
        let kind = match value {
            "fn" => TokenKind::Fn,         // Legacy: kept for backward compatibility
            "def" => TokenKind::Def,       // Pythonic function definition
            "lambda" => TokenKind::Lambda, // Pythonic lambda expression
            "let" => TokenKind::Let,       // Optional: kept for backward compatibility
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "elif" => TokenKind::Elif,
            "for" => TokenKind::For,
            "while" => TokenKind::While,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "in" => TokenKind::In,
            "is" => TokenKind::Is,
            "not" => TokenKind::Not,
            "use" => TokenKind::Use,
            "from" => TokenKind::From,
            "as" => TokenKind::As,
            "pub" => TokenKind::Pub,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "spawn" => TokenKind::Spawn,
            "match" => TokenKind::Match,
            "case" => TokenKind::Case,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "print" => TokenKind::Print,
            "pass" => TokenKind::Pass,
            "None" | "none" => TokenKind::None,
            "try" => TokenKind::Try,
            "except" => TokenKind::Except,
            "finally" => TokenKind::Finally,
            "raise" => TokenKind::Raise,
            _ => TokenKind::Identifier(value.to_string()),
        };

        self.emit_token(kind, start, self.offset - start);
    }

    fn tokenize_unicode_identifier(&mut self) {
        let start = self.offset;

        while let Some(ch) = self.current_char() {
            if ch.is_ascii_alphanumeric() || ch == b'_' || (ch > 127) {
                self.advance(1);
            } else {
                break;
            }
        }

        let value = unsafe { std::str::from_utf8_unchecked(&self.source[start..self.offset]) };
        self.emit_token(
            TokenKind::UnicodeIdentifier(value.to_string()),
            start,
            self.offset - start,
        );
    }

    fn skip_to_end_of_line(&mut self) {
        while let Some(ch) = self.current_char() {
            if ch == b'\n' {
                self.emit_token(TokenKind::Newline, self.offset, 1);
                self.advance(1);
                return;
            }
            self.advance(1);
        }
        // EOF reached
    }

    fn finalize_indentation(&mut self) {
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            let span = Span::new(self.offset, self.offset);
            self.tokens.push(Token::new(TokenKind::Dedent, span));
        }

        let eof_span = Span::new(self.offset, self.offset);
        self.tokens.push(Token::new(TokenKind::Eof, eof_span));
    }
}

// Legacy function for backward compatibility - delegates to new implementation
pub fn tokenize_legacy(source: &str) -> LexResult<Vec<Token>> {
    tokenize(source)
}
