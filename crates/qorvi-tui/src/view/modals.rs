//! Diálogos centrados: nueva tarea, confirmación y ayuda.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{bold, centered, fg, tail, text_width, truncate, wrap};
use crate::state::{AppState, Modal};
use crate::theme::{color, Token};

const HELP: [(&str, &str); 14] = [
    ("Alt+h/j/k/l", "mover el foco entre paneles"),
    ("Alt+1..9", "ir a la tarea N"),
    ("Alt+n", "nueva tarea"),
    ("Alt+d", "review / diff"),
    ("Alt+z", "zoom del panel"),
    ("Alt+t", "mostrar/ocultar la terminal"),
    ("Alt+m", "ver de nuevo el último aviso"),
    ("Alt+q", "salir"),
    ("F1", "esta ayuda"),
    ("F5..F8", "DEMO: trabajando/permiso/listo/error"),
    ("Enter", "enviar mensaje o confirmar permiso"),
    ("Alt+Enter", "salto de línea en el mensaje"),
    ("Esc", "cancelar el turno / cerrar diálogo"),
    ("↑/↓", "elegir opción de un permiso"),
];

fn dialog<'a>(title: &str, area: Rect, token: Token) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(fg(token))
        .style(Style::default().bg(color(Token::Surface)))
        .title(Span::styled(
            truncate(title, usize::from(area.width.saturating_sub(2))),
            bold(token),
        ))
}

pub fn render(state: &AppState, frame: &mut Frame, area: Rect) {
    match &state.modal {
        Modal::None | Modal::Review { .. } => {}
        Modal::NewTask {
            name,
            agent_idx,
            base,
            field,
        } => new_task(state, frame, area, name, *agent_idx, base, *field),
        Modal::Confirm { message, .. } => confirm(frame, area, message),
        Modal::Help => help(frame, area),
    }
}

fn new_task(
    state: &AppState,
    frame: &mut Frame,
    area: Rect,
    name: &str,
    agent_idx: usize,
    base: &str,
    field: usize,
) {
    let rect = centered(area, 58, 11);
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = dialog(" Nueva tarea ", rect, Token::Accent);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let w = usize::from(inner.width);
    let label_w = 11;
    let cursor = if state.ascii { "_" } else { "▏" };
    let row = |idx: usize, label: &str, value: String| -> Line<'static> {
        let on = idx == field;
        let mark = if on {
            if state.ascii {
                ">"
            } else {
                "›"
            }
        } else {
            " "
        };
        let head = format!("{mark}{label:<width$}", width = label_w - 1);
        let room = w.saturating_sub(label_w);
        let value = tail(&value, room);
        let style = if on {
            bold(Token::Accent)
        } else {
            fg(Token::Text)
        };
        Line::from(vec![
            Span::styled(
                head,
                if on {
                    bold(Token::Accent)
                } else {
                    fg(Token::Muted)
                },
            ),
            Span::styled(value, style),
        ])
    };
    let agent = state.agents.get(agent_idx).map_or("-", String::as_str);
    let lines = vec![
        Line::raw(""),
        row(
            0,
            "Nombre",
            format!("{name}{}", if field == 0 { cursor } else { "" }),
        ),
        row(1, "Agente", format!("‹ {agent} ›")),
        row(
            2,
            "Rama base",
            format!("{base}{}", if field == 2 { cursor } else { "" }),
        ),
        Line::raw(""),
        Line::styled(
            truncate("Tab/Shift+Tab: campo · ←/→: agente", w),
            fg(Token::Muted),
        ),
        Line::styled(
            truncate("Enter: crear · Esc: cancelar", w),
            fg(Token::Muted),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn confirm(frame: &mut Frame, area: Rect, message: &str) {
    let w = area.width.min(54);
    let inner_w = usize::from(w.saturating_sub(4)).max(1);
    let body = wrap(message, inner_w);
    let rect = centered(area, w, body.len() as u16 + 5);
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = dialog(" Confirmar ", rect, Token::Warning);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines: Vec<Line> = vec![Line::raw("")];
    lines.extend(
        body.into_iter()
            .map(|l| Line::styled(format!(" {l}"), fg(Token::Text))),
    );
    lines.push(Line::raw(""));
    lines.push(Line::styled(" [s] Sí · [n] No", bold(Token::Warning)));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn help(frame: &mut Frame, area: Rect) {
    let rect = centered(area, 64, HELP.len() as u16 + 4);
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = dialog(" Ayuda · atajos (Esc cierra) ", rect, Token::Accent);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let w = usize::from(inner.width);
    let mut lines: Vec<Line> = vec![Line::raw("")];
    lines.extend(HELP.iter().map(|(k, d)| {
        let key = format!(" {k:<12}");
        let rest = w.saturating_sub(text_width(&key));
        Line::from(vec![
            Span::styled(key, bold(Token::Accent)),
            Span::styled(truncate(d, rest), fg(Token::Text)),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), inner);
}
