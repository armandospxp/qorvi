//! Panel del explorador de archivos (árbol falso de la demo).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::panes::pane_block;
use super::{bold, fg, text_width, truncate};
use crate::explorer::{visible_rows, Row, RowKind};
use crate::state::{AppState, ChangeMark};
use crate::theme::{color, Token};

fn mark_token(mark: ChangeMark) -> Token {
    match mark {
        ChangeMark::Modified => Token::Info,
        ChangeMark::Added => Token::Accent,
        ChangeMark::Deleted => Token::Error,
        // `warning` se reserva para lo que pide atención del usuario.
        ChangeMark::Untracked => Token::Muted,
    }
}

fn row_line(row: &Row, selected: bool, focused: bool, ascii: bool, width: usize) -> Line<'static> {
    let cursor = match (selected, ascii) {
        (true, true) => ">",
        (true, false) => "›",
        (false, _) => " ",
    };
    let indent = " ".repeat(row.depth * 2);
    let (glyph, glyph_style, name, name_style, tail) = match row.kind {
        RowKind::Dir { expanded, changed } => {
            let glyph = match (expanded, ascii) {
                (true, false) => "▾",
                (false, false) => "▸",
                (true, true) => "v",
                (false, true) => ">",
            };
            let tail = match (changed, ascii) {
                (false, _) => "",
                (true, false) => " •",
                (true, true) => " *",
            };
            (
                glyph.to_string(),
                fg(Token::Muted),
                format!("{}/", row.name),
                fg(Token::Text),
                tail,
            )
        }
        RowKind::File { mark } => {
            let letter = mark.map_or(' ', ChangeMark::letter);
            let name_style = match mark {
                Some(ChangeMark::Deleted) => fg(Token::Muted).add_modifier(Modifier::CROSSED_OUT),
                _ => fg(Token::Text),
            };
            (
                letter.to_string(),
                mark.map_or_else(|| fg(Token::Text), |m| fg(mark_token(m))),
                row.name.clone(),
                name_style,
                "",
            )
        }
    };
    let head = format!("{cursor}{indent}");
    let used = text_width(&head) + 2 + text_width(tail);
    let name = truncate(&name, width.saturating_sub(used));
    let spans = vec![
        Span::styled(head, fg(Token::Accent)),
        Span::styled(format!("{glyph} "), glyph_style),
        Span::styled(name, name_style),
        Span::styled(tail, fg(Token::Muted)),
    ];
    let mut line = Line::from(spans);
    if selected {
        let bg = Style::default().bg(color(Token::SurfaceActive));
        line = line.style(if focused {
            bg.patch(bold(Token::Accent))
        } else {
            bg
        });
    }
    line
}

pub fn render(state: &AppState, frame: &mut Frame, area: Rect, focused: bool) {
    let block = pane_block(" Archivos ", focused, area.width);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(task) = state.active_task() else {
        return;
    };
    let rows = visible_rows(&task.files, &task.explorer.expanded);
    if rows.is_empty() || inner.width == 0 || inner.height == 0 {
        return;
    }
    let selected = task.explorer.selected.min(rows.len() - 1);
    let h = usize::from(inner.height);
    // Desplaza la ventana lo justo para que la fila seleccionada quede a la vista.
    let start = (selected + 1).saturating_sub(h);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(h)
        .map(|(i, row)| {
            row_line(
                row,
                i == selected,
                focused,
                state.ascii,
                usize::from(inner.width),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
