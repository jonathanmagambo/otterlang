use common::Span;
use utils::errors::{Diagnostic, DiagnosticSeverity};

use super::TypeError;

/// Convert type checker errors into rich diagnostics with span guessing and suggestions.
pub fn from_type_errors(errors: &[TypeError], source_id: &str, source: &str) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| to_diagnostic(error, source_id, source))
        .collect()
}

fn to_diagnostic(error: &TypeError, source_id: &str, source: &str) -> Diagnostic {
    let span = error.span.unwrap_or_else(|| guess_span(error, source));
    let mut diagnostic = Diagnostic::new(
        DiagnosticSeverity::Error,
        source_id.to_string(),
        span,
        error.message.clone(),
    );

    if let Some(hint) = &error.hint {
        diagnostic = diagnostic.with_suggestion(hint.clone());
    }

    if let Some(help) = &error.help {
        diagnostic = diagnostic.with_help(help.clone());
    }

    diagnostic
}

fn guess_span(error: &TypeError, source: &str) -> Span {
    let candidates = extract_candidates(&error.message);

    for candidate in candidates {
        if let Some(span) = find_identifier_span(source, candidate) {
            return span;
        }
    }

    Span::new(0, 0)
}

fn extract_candidates(message: &str) -> Vec<&str> {
    let mut candidates = Vec::new();

    // Backtick enclosed identifiers
    candidates.extend(
        message
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|segment| !segment.trim().is_empty()),
    );

    // Single-quoted identifiers
    candidates.extend(
        message
            .split('\'')
            .skip(1)
            .step_by(2)
            .filter(|segment| !segment.trim().is_empty()),
    );

    // After colon (e.g., "undefined variable: foo")
    if let Some(idx) = message.find(':') {
        let candidate = message[idx + 1..]
            .split_whitespace()
            .next()
            .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'));
        if let Some(candidate) = candidate
            && !candidate.is_empty() {
                candidates.push(candidate);
            }
    }

    candidates
}

fn find_identifier_span(source: &str, needle: &str) -> Option<Span> {
    if needle.is_empty() {
        return None;
    }

    let bytes = source.as_bytes();
    let needle_bytes = needle.as_bytes();
    let len = needle_bytes.len();
    let mut byte_index = 0usize;

    while byte_index + len <= bytes.len() {
        if &bytes[byte_index..byte_index + len] == needle_bytes
            && is_word_boundary(bytes, byte_index, len)
        {
            return Some(Span::new(byte_index, byte_index + len));
        }

        // Advance by one character (respect UTF-8)
        if let Some(ch) = source[byte_index..].chars().next() {
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }

    None
}

fn is_word_boundary(bytes: &[u8], start: usize, len: usize) -> bool {
    let is_ident_char = |b: u8| -> bool {
        let ch = b as char;
        ch.is_alphanumeric() || ch == '_'
    };

    let start_ok = if start == 0 {
        true
    } else {
        !is_ident_char(bytes[start - 1])
    };

    let end_index = start + len;
    let end_ok = if end_index >= bytes.len() {
        true
    } else {
        !is_ident_char(bytes[end_index])
    };

    start_ok && end_ok
}
