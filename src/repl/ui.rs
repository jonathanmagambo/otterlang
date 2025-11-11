//! UI rendering for the TUI REPL

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::repl::state::{AppState, Mode, OutputKind};

/// Draw the main UI
pub fn draw_ui(frame: &mut Frame, state: &mut AppState) {
    let size = frame.area();

    // Create main layout: sidebar (30%) and main area (70%)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(size);

    let main_area = chunks[0];
    let sidebar_area = chunks[1];

    // Split main area into output and input
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)])
        .split(main_area);

    let output_area = main_chunks[0];
    let input_area = main_chunks[1];

    // Draw components
    draw_output(frame, state, output_area);
    draw_input(frame, state, input_area);
    draw_history(frame, state, sidebar_area);
    draw_status_bar(frame, state, size);

    // Draw help overlay if needed
    if state.show_help {
        draw_help_overlay(frame, size);
    }
}

/// Draw the output area
fn draw_output(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = state
        .output
        .iter()
        .rev()
        .skip(state.output_scroll)
        .take(area.height as usize - 2)
        .rev()
        .map(|entry| {
            let style = match entry.kind {
                OutputKind::Input => Style::default().fg(Color::Cyan),
                OutputKind::Output => Style::default().fg(Color::Green),
                OutputKind::Error => Style::default().fg(Color::Red),
                OutputKind::Info => Style::default().fg(Color::Yellow),
            };

            let mut text = if let Some(ts) = &entry.timestamp {
                format!("[{}] ", ts)
            } else {
                String::new()
            };
            text.push_str(&entry.content);

            ListItem::new(Line::from(vec![Span::styled(text, style)]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title("Output")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        );

    frame.render_widget(list, area);

    // Draw scrollbar
    if state.output.len() > (area.height as usize - 2) {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(state.output.len())
            .viewport_content_length((area.height as usize - 2).max(1))
            .position(state.output_scroll);
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Draw the input area
fn draw_input(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let lines = state.input_lines();
    let prompt = if state.continuation_mode {
        "  ... "
    } else {
        "otter> "
    };

    let styled_lines: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let mut spans = vec![];
            
            // Add prompt for first line
            if i == 0 {
                spans.push(Span::styled(
                    prompt,
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    "      ",
                    Style::default(),
                ));
            }

            // Add line content with cursor
            let line_chars: Vec<char> = line.chars().collect();
            for (j, ch) in line_chars.iter().enumerate() {
                if i == state.cursor.0 && j == state.cursor.1 {
                    // Cursor position - will be handled by terminal cursor
                    spans.push(Span::styled(
                        ch.to_string(),
                        Style::default().bg(Color::White).fg(Color::Black),
                    ));
                } else {
                    spans.push(Span::styled(
                        ch.to_string(),
                        Style::default(),
                    ));
                }
            }

            // Show cursor if at end of line
            if i == state.cursor.0 && state.cursor.1 == line.len() {
                spans.push(Span::styled(
                    " ",
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(styled_lines)
        .block(
            Block::default()
                .title("Input")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(paragraph, area);

    // Set cursor position
    let cursor_x = area.x
        + prompt.len() as u16
        + state.cursor.1.min(lines.get(state.cursor.0).map(|l| l.len()).unwrap_or(0)) as u16
        + 1; // +1 for border
    let cursor_y = area.y + state.cursor.0 as u16 + 1; // +1 for border
    frame.set_cursor_position(ratatui::layout::Position::new(cursor_x, cursor_y));
}

/// Draw the history sidebar
fn draw_history(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = state
        .history
        .iter()
        .rev()
        .skip(state.history_scroll)
        .take(area.height as usize - 2)
        .rev()
        .enumerate()
        .map(|(i, cmd)| {
            let idx = state.history.len() - state.history_scroll - (area.height as usize - 2) + i;
            let is_selected = state.history_index.map(|hi| hi == idx).unwrap_or(false);
            
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            let text = format!("{:4}: {}", idx + 1, cmd);
            ListItem::new(Line::from(vec![Span::styled(text, style)]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title("History")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );

    frame.render_widget(list, area);

    // Draw scrollbar
    if state.history.len() > (area.height as usize - 2) {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(state.history.len())
            .viewport_content_length((area.height as usize - 2).max(1))
            .position(state.history_scroll);
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Draw the status bar
fn draw_status_bar(frame: &mut Frame, state: &AppState, area: Rect) {
    let status_area = Rect {
        x: area.x,
        y: area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    let mode_text = match state.mode {
        Mode::Input => "INPUT",
        Mode::History => "HISTORY",
        Mode::Help => "HELP",
    };

    let status_text = format!(
        " {} | Errors: {} | {} | Ctrl+C: Clear | Ctrl+D: Exit | F1: Help",
        mode_text,
        state.error_count,
        if state.continuation_mode {
            "MULTI-LINE"
        } else {
            "SINGLE-LINE"
        }
    );

    let paragraph = Paragraph::new(status_text)
        .style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(paragraph, status_area);
}

/// Draw help overlay
fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(60, 60, area);

    let help_text = vec![
        Line::from("OtterLang REPL - Keyboard Shortcuts"),
        Line::from(""),
        Line::from("Navigation:"),
        Line::from("  ↑/↓          Navigate history"),
        Line::from("  Ctrl+↑/↓     Scroll output"),
        Line::from("  ←/→          Move cursor"),
        Line::from(""),
        Line::from("Input:"),
        Line::from("  Enter        Execute input"),
        Line::from("  Ctrl+Enter   Force execute (multi-line)"),
        Line::from("  Tab          Insert 4 spaces"),
        Line::from("  Esc          Clear input / Cancel"),
        Line::from(""),
        Line::from("Commands:"),
        Line::from("  Ctrl+C       Clear current input"),
        Line::from("  Ctrl+D       Exit REPL"),
        Line::from("  Ctrl+L       Clear output"),
        Line::from("  F1           Toggle this help"),
        Line::from(""),
        Line::from("Press F1 or Esc to close"),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(ratatui::widgets::Wrap { trim: true });

    frame.render_widget(paragraph, popup_area);
}

/// Helper to create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

