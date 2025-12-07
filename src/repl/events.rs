//! Event handling for the TUI REPL

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Terminal events
#[derive(Debug, Clone, Copy)]
pub enum AppEvent {
    /// Key press event
    Key(KeyEvent),
    /// Terminal resize event (width, height)
    Resize(u16, u16),
    /// Tick event (for periodic updates)
    Tick,
}

/// Event handler for terminal events
pub struct EventHandler {
    rx: mpsc::Receiver<AppEvent>,
    _tx: mpsc::Sender<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_tx = tx.clone();

        let initial_tx = tx.clone();
        let _ = initial_tx.send(AppEvent::Tick);

        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));

                if let Ok(true) = event::poll(timeout)
                    && let Ok(evt) = event::read()
                {
                    match evt {
                        Event::Key(key) => {
                            if key.kind == KeyEventKind::Press {
                                let _ = event_tx.send(AppEvent::Key(key));
                            }
                        }
                        Event::Resize(w, h) => {
                            let _ = event_tx.send(AppEvent::Resize(w, h));
                        }
                        _ => {}
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    if event_tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Get the next event (non-blocking)
    pub fn next(&self) -> Result<AppEvent> {
        self.rx
            .recv()
            .map_err(|e| anyhow::anyhow!("Failed to receive event: {}", e))
    }

    /// Try to get the next event (non-blocking)
    #[expect(dead_code, reason = "Work in progress")]
    pub fn try_next(&self) -> Option<AppEvent> {
        self.rx.try_recv().ok()
    }
}

/// Helper to check if a key event matches a specific key combination
pub fn matches_key(key: &KeyEvent, code: KeyCode, modifiers: KeyModifiers) -> bool {
    key.code == code && key.modifiers == modifiers
}

/// Helper to check if Ctrl is pressed
pub fn is_ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Helper to check if Alt is pressed
#[expect(dead_code, reason = "Work in progress")]
pub fn is_alt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
}

/// Helper to check if Shift is pressed
#[expect(dead_code, reason = "Work in progress")]
pub fn is_shift(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
}
