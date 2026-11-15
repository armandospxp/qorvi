//! Vista de review: archivos cambiados a la izquierda, diff unificado a la derecha.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{bold, fg, truncate};
use crate::state::{AppState, ChangedFile};
use crate::theme::{color, Token};

fn diff_line(l: &str, width: usize) -> Line<'static> {
    let style = if l.starts_with("+++") || l.starts_with("---") {
        fg(Token::Muted).add_modifier(Modifier::BOLD)
    } else if l.starts_with("@@") {
        fg(Token::Info)
    } else if l.starts_with('+') {
        fg(Token::Accent)
    } else if l.starts_with('-') {
        fg(Token::Error)
    } else {
        fg(Token::Text)
    };
    Line::styled(truncate(l, width), style)
}

pub fn render(
    state: &AppState,
    frame: &mut Frame,
    area: Rect,
    files: &[ChangedFile],
    selected: usize,
) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    frame.render_widget(Clear, area);
    let name = state.active_task().map_or("-", |t| t.name.as_str());
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(fg(Token::Accent))
        .style(ratatui::style::Style::default().bg(color(Token::Background)))
        .title(Span::styled(
            truncate(
                &format!(" Review · {name} · {} archivos ", files.len()),
                usize::from(area.width.saturating_sub(2)),
            ),
            bold(Token::Accent),
        ));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let [main, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let [list_area, diff_area] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(main);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(fg(Token::Border))
        .title(Span::styled(" Archivos ", fg(Token::Muted)));
    let list_inner = list_block.inner(list_area);
    frame.render_widget(list_block, list_area);
    let lw = usize::from(list_inner.width);
    let rows: Vec<Line> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let on = i == selected;
            let mark = if on {
                if state.ascii {
                    ">"
                } else {
                    "›"
                }
            } else {
                " "
            };
            let text = truncate(&format!("{mark}{} {}", f.kind.letter(), f.path), lw);
            if on {
                Line::styled(text, bold(Token::Accent).bg(color(Token::SurfaceActive)))
            } else {
                Line::styled(text, fg(Token::Text))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), list_inner);

    let (title, diff): (String, &[String]) = match files.get(selected) {
        Some(f) => (format!(" {} · {} ", f.path, f.kind.label()), &f.diff),
        None => (" Diff ".to_string(), &[]),
    };
    let diff_block = Block::default()
        .borders(Borders::ALL)
        .border_style(fg(Token::Border))
        .title(Span::styled(
            truncate(&title, usize::from(diff_area.width.saturating_sub(2))),
            fg(Token::Muted),
        ));
    let diff_inner = diff_block.inner(diff_area);
    frame.render_widget(diff_block, diff_area);
    let dw = usize::from(diff_inner.width);
    let lines: Vec<Line> = diff
        .iter()
        .take(usize::from(diff_inner.height))
        .map(|l| diff_line(l, dw))
        .collect();
    frame.render_widget(Paragraph::new(lines), diff_inner);

    frame.render_widget(
        Paragraph::new(Line::styled(
            truncate(
                " j/k mover · c commit · m merge · p push+PR · x descartar · Esc cerrar",
                usize::from(footer.width),
            ),
            fg(Token::Muted),
        )),
        footer,
    );
}
