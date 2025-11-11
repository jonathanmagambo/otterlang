//! REPL (Read-Eval-Print Loop) for OtterLang
//!
//! Provides an interactive shell for executing OtterLang code incrementally.

mod engine;
mod events;
mod state;
mod tui;
mod ui;

pub use engine::{EvaluationKind, EvaluationResult, ReplEngine};
pub use state::{AppState, Mode, OutputKind, OutputEntry};
pub use tui::Tui;
