//! Paneles de editor y terminal (texto fijo en la demo) y selección de panel.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::{chat, explorer, fg, truncate};
use crate::layout;
use crate::state::{shortcut_label, AppState, Pane};
use crate::theme::{color, status_label, status_symbol, Token};

pub fn pane_block(title: &str, focused: bool, width: u16) -> Block<'static> {
    let border = if focused {
        Token::Accent
    } else {
        Token::Border
    };
    let title_style = if focused {
        super::bold(Token::Accent)
    } else {
        fg(Token::Muted)
    };
    let title = truncate(title, usize::from(width.saturating_sub(2)));
    Block::default()
        .borders(Borders::ALL)
        .border_style(fg(border))
        .title(Span::styled(title, title_style))
}

pub fn render_pane(state: &AppState, frame: &mut Frame, area: Rect, pane: Pane) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focused = state.focus == pane;
    match pane {
        Pane::Explorer => explorer::render(state, frame, area, focused),
        Pane::Editor => render_editor(state, frame, area, focused),
        Pane::Terminal => render_terminal(state, frame, area, focused),
        Pane::Agent => chat::render(state, frame, area, focused),
    }
}

fn render_editor(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let title = match state.active_task().and_then(|t| t.editor_file.as_deref()) {
        Some(file) => format!(" nvim · {file} "),
        None => " nvim ".to_string(),
    };
    let block = pane_block(&title, focused, area.width);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(task) = state.active_task() else {
        return;
    };
    let width = usize::from(inner.width);
    let lines: Vec<Line> = task
        .editor_lines
        .iter()
        .take(usize::from(inner.height))
        .map(|l| {
            let l = truncate(l, width);
            if l.trim_start().starts_with('~') {
                return Line::styled(l, fg(Token::Muted));
            }
            let split = l.char_indices().nth(4).map_or(l.len(), |(i, _)| i);
            let (gutter, code) = l.split_at(split);
            Line::from(vec![
                Span::styled(gutter.to_string(), fg(Token::Muted)),
                Span::styled(code.to_string(), fg(Token::Text)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_terminal(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let block = pane_block(" zsh ", focused, area.width);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(task) = state.active_task() else {
        return;
    };
    let width = usize::from(inner.width);
    let h = usize::from(inner.height);
    let start = task.terminal_lines.len().saturating_sub(h);
    let lines: Vec<Line> = task.terminal_lines[start..]
        .iter()
        .map(|l| {
            let l = truncate(l, width);
            if l.starts_with('$') {
                Line::styled(l, fg(Token::Accent))
            } else {
                Line::styled(l, fg(Token::Text))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn render_minimized_terminal(state: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focused = state.focus == Pane::Terminal;
    let style = if focused {
        super::bold(Token::Accent)
    } else {
        fg(Token::Muted)
    };
    let text = truncate(
        &format!(
            " ▸ Terminal (minimizada) · {} para desplegar",
            shortcut_label(state.shortcut_style, "t")
        ),
        usize::from(area.width),
    );
    frame.render_widget(Paragraph::new(Line::styled(text, style)), area);
}

pub fn render_tabs(state: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (lead, sep, tight) = layout::tab_chrome(area.width);
    let mut spans = vec![Span::raw(lead)];
    for (i, pane) in [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal]
        .into_iter()
        .enumerate()
    {
        if i > 0 {
            spans.push(Span::styled(sep, fg(Token::Border)));
        }
        let style = if pane == state.focus {
            Style::default()
                .fg(color(Token::Background))
                .bg(color(Token::Accent))
                .add_modifier(Modifier::BOLD)
        } else {
            fg(Token::Muted)
        };
        spans.push(Span::styled(layout::tab_label(pane, tight), style));
    }
    if let Some(task) = state.active_task() {
        spans.push(Span::styled(
            format!(
                "   {} {}",
                status_symbol(task.status, state.ascii),
                status_label(task.status)
            ),
            fg(Token::Muted),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
