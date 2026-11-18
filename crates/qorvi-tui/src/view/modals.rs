//! Diálogos centrados: nueva tarea, confirmación y ayuda.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{bold, centered, fg, tail, text_width, truncate, wrap};
use crate::shortcuts::{direct_label, prefix_label, SHORTCUTS};
use crate::state::{AppState, Modal, ShortcutStyle};
use crate::theme::{color, Token};

/// Teclas que no dependen de la forma de atajo.
/// La primera (F5..F8) solo existe en la demo.
const HELP_FIXED: [(&str, &str); 9] = [
    ("F5..F8", "DEMO: trabajando/permiso/listo/error"),
    ("Enter", "enviar mensaje o confirmar permiso"),
    ("Alt+Enter", "salto de línea en el mensaje"),
    ("Esc", "cancelar el turno / cerrar diálogo"),
    ("↑/↓", "elegir opción de un permiso"),
    ("Shift+Tab", "cambiar el modo de permisos del agente"),
    ("Enter", "editor/terminal: reabrir el programa si terminó"),
    ("j/k ↑/↓", "explorador: mover la selección"),
    (
        "Enter/l h r",
        "explorador: abrir · contraer/subir · refrescar",
    ),
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
        Modal::Help => help(state, frame, area),
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

/// Rectángulo del diálogo de ayuda dentro de `area`.
pub fn help_rect(area: Rect) -> Rect {
    centered(area, 86, (SHORTCUTS.len() + HELP_FIXED.len()) as u16 + 6)
}

fn help(state: &AppState, frame: &mut Frame, area: Rect) {
    // Dos columnas: la forma que muestra el estilo y la otra. Las acciones sin atajo
    // directo solo tienen el prefijo.
    let rows: Vec<(String, String, &str)> = SHORTCUTS
        .iter()
        .map(|s| {
            let prefix = prefix_label(s);
            let direct = s.direct.map(|c| direct_label(state.direct_mod, c));
            match (state.shortcut_style, direct) {
                (ShortcutStyle::Direct, Some(d)) => (d, prefix, s.desc),
                (ShortcutStyle::Prefix, Some(d)) => (prefix, d, s.desc),
                (_, None) => (prefix, String::new(), s.desc),
            }
        })
        .chain(
            HELP_FIXED
                .iter()
                .skip(usize::from(!state.demo))
                .map(|(k, d)| (k.to_string(), String::new(), *d)),
        )
        .collect();
    let rect = help_rect(area);
    if rect.width < 4 || rect.height < 3 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = dialog(" Ayuda · atajos (Esc cierra) ", rect, Token::Accent);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let w = usize::from(inner.width);
    let col1 = rows
        .iter()
        .map(|(k, _, _)| text_width(k))
        .max()
        .unwrap_or(0)
        + 2;
    let col2 = rows
        .iter()
        .map(|(_, k, _)| text_width(k))
        .max()
        .unwrap_or(0)
        + 2;
    let mut lines: Vec<Line> = vec![
        Line::styled(
            truncate(" También: Alt+<tecla> equivale a Ctrl+Space <tecla>", w),
            fg(Token::Muted),
        ),
        Line::raw(""),
    ];
    lines.extend(rows.iter().map(|(k1, k2, d)| {
        let first = format!(" {k1:<width$}", width = col1 - 1);
        let second = format!("{k2:<col2$}");
        let rest = w.saturating_sub(text_width(&first) + text_width(&second));
        Line::from(vec![
            Span::styled(first, bold(Token::Accent)),
            Span::styled(second, fg(Token::Muted)),
            Span::styled(truncate(d, rest), fg(Token::Text)),
        ])
    }));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        truncate(" Ratón: clic para enfocar · rueda para desplazar", w),
        fg(Token::Muted),
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}
