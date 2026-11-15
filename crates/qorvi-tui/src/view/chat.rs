//! Panel del agente: historial, tool calls, permisos inline y caja de entrada.

use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::panes::pane_block;
use super::{bold, fg, truncate, wrap};
use crate::state::{AppState, ChatItem, Modal, Task, ToolStatus};
use crate::theme::{color, status_label, status_symbol, Token};

fn tool_status(status: ToolStatus, ascii: bool) -> (&'static str, &'static str, Token) {
    match (status, ascii) {
        (ToolStatus::Pending, false) => ("…", "pendiente", Token::Muted),
        (ToolStatus::InProgress, false) => ("●", "en curso", Token::Info),
        (ToolStatus::Completed, false) => ("✓", "completado", Token::Accent),
        (ToolStatus::Failed, false) => ("✗", "fallido", Token::Error),
        (ToolStatus::Pending, true) => ("~", "pendiente", Token::Muted),
        (ToolStatus::InProgress, true) => ("*", "en curso", Token::Info),
        (ToolStatus::Completed, true) => ("+", "completado", Token::Accent),
        (ToolStatus::Failed, true) => ("x", "fallido", Token::Error),
    }
}

fn wrapped(
    out: &mut Vec<Line<'static>>,
    first: &str,
    rest: &str,
    text: &str,
    width: usize,
    style: Style,
    prefix_style: Style,
) {
    let indent = super::text_width(first);
    for (i, l) in wrap(text, width.saturating_sub(indent))
        .into_iter()
        .enumerate()
    {
        let prefix = if i == 0 { first } else { rest };
        out.push(Line::from(vec![
            Span::styled(prefix.to_string(), prefix_style),
            Span::styled(l, style),
        ]));
    }
}

pub fn history_lines(task: &Task, width: usize, ascii: bool) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for item in &task.chat {
        if !out.is_empty() {
            out.push(Line::raw(""));
        }
        match item {
            ChatItem::User(t) => {
                let p = if ascii { "> " } else { "› " };
                wrapped(
                    &mut out,
                    p,
                    "  ",
                    t,
                    width,
                    fg(Token::Text),
                    bold(Token::Accent),
                );
            }
            ChatItem::Agent(t) => {
                wrapped(
                    &mut out,
                    "  ",
                    "  ",
                    t,
                    width,
                    fg(Token::Text),
                    fg(Token::Text),
                );
            }
            ChatItem::ToolCall {
                title,
                kind,
                status,
                expanded,
            } => {
                let fold = match (*expanded, ascii) {
                    (true, false) => "▾",
                    (false, false) => "▸",
                    (true, true) => "v",
                    (false, true) => ">",
                };
                let (sym, label, token) = tool_status(*status, ascii);
                let head = truncate(
                    &format!("{fold} {title} ({kind}) "),
                    width.saturating_sub(2),
                );
                let tail = truncate(
                    &format!("{sym} {label}"),
                    width.saturating_sub(super::text_width(&head)),
                );
                out.push(Line::from(vec![
                    Span::styled(head, fg(Token::Muted)),
                    Span::styled(tail, fg(token)),
                ]));
                if *expanded {
                    out.push(Line::styled(
                        truncate(&format!("    tipo: {kind} · estado: {label}"), width),
                        fg(Token::Muted),
                    ));
                }
            }
            ChatItem::Permission {
                title,
                options,
                selected,
                resolved,
            } => {
                let sym = status_symbol(crate::state::AgentStatus::NeedsPermission, ascii);
                match resolved {
                    None => {
                        out.push(Line::styled(
                            truncate(&format!("{sym} Permiso: {title}"), width),
                            bold(Token::Warning),
                        ));
                        for (i, o) in options.iter().enumerate() {
                            let on = i == *selected;
                            let mark = match (on, ascii) {
                                (true, false) => "› (•) ",
                                (false, false) => "  ( ) ",
                                (true, true) => "> (*) ",
                                (false, true) => "  ( ) ",
                            };
                            let style = if on {
                                bold(Token::Warning).bg(color(Token::SurfaceActive))
                            } else {
                                fg(Token::Text)
                            };
                            out.push(Line::styled(truncate(&format!("{mark}{o}"), width), style));
                        }
                        out.push(Line::styled(
                            truncate("  ↑/↓ elegir · Enter confirmar", width),
                            fg(Token::Muted),
                        ));
                    }
                    Some(r) => {
                        let opt = options.get(*r).map_or("?", String::as_str);
                        let ok = status_symbol(crate::state::AgentStatus::Done, ascii);
                        out.push(Line::styled(
                            truncate(&format!("{ok} Permiso: {title} → {opt}"), width),
                            fg(Token::Muted),
                        ));
                    }
                }
            }
            ChatItem::Notice(t) => {
                wrapped(
                    &mut out,
                    "— ",
                    "  ",
                    t,
                    width,
                    fg(Token::Muted),
                    fg(Token::Muted),
                );
            }
        }
    }
    out
}

pub fn render(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let Some(task) = state.active_task() else {
        let block = pane_block(" Agente ", focused, area.width);
        frame.render_widget(block, area);
        return;
    };
    let title = format!(
        " Agente · {} · {} {} ",
        task.agent,
        status_symbol(task.status, state.ascii),
        status_label(task.status)
    );
    let block = pane_block(&title, focused, area.width);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = usize::from(inner.width);
    let input_lines = wrap(&task.input, width.saturating_sub(2));
    let want = (input_lines.len() as u16).saturating_add(2).clamp(3, 6);
    let input_h = want.min(inner.height.saturating_sub(1));
    let [hist_area, input_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(input_h)]).areas(inner);

    let lines = history_lines(task, width, state.ascii);
    let h = usize::from(hist_area.height);
    let start = lines.len().saturating_sub(h);
    frame.render_widget(Paragraph::new(lines[start..].to_vec()), hist_area);

    if input_area.height == 0 {
        return;
    }
    let border = if focused {
        Token::Accent
    } else {
        Token::Border
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(fg(border))
        .title(Span::styled(
            truncate(
                " Mensaje · Enter envía · Alt+Enter salto ",
                usize::from(input_area.width.saturating_sub(2)),
            ),
            fg(Token::Muted),
        ));
    let box_inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);
    if box_inner.width == 0 || box_inner.height == 0 {
        return;
    }
    if task.input.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                truncate("Escribe un mensaje…", usize::from(box_inner.width)),
                fg(Token::Muted),
            )),
            box_inner,
        );
    } else {
        let w = usize::from(box_inner.width);
        let lines = wrap(&task.input, w);
        let bh = usize::from(box_inner.height);
        let start = lines.len().saturating_sub(bh);
        let shown: Vec<Line> = lines[start..]
            .iter()
            .map(|l| Line::styled(truncate(l, w), fg(Token::Text)))
            .collect();
        frame.render_widget(Paragraph::new(shown), box_inner);
    }
    if focused && state.modal == Modal::None {
        let w = usize::from(box_inner.width);
        let lines = wrap(&task.input, w);
        let bh = usize::from(box_inner.height);
        let start = lines.len().saturating_sub(bh);
        let row = (lines.len() - start).saturating_sub(1) as u16;
        let col = lines.last().map_or(0, |l| super::text_width(l)) as u16;
        let x = (box_inner.x + col).min(box_inner.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(x, box_inner.y + row));
    }
}
