//! Barra de estado de 1 fila.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{fg, text_width, truncate};
use crate::shortcuts::shortcut_label;
use crate::state::AppState;
use crate::theme::{color, status_label, status_symbol, status_token, Token};

struct Segment {
    text: String,
    style: Style,
    /// 0 = nunca se descarta; mayor = se descarta antes.
    drop_rank: u8,
}

fn seg(text: impl Into<String>, style: Style, drop_rank: u8) -> Segment {
    Segment {
        text: text.into(),
        style,
        drop_rank,
    }
}

pub fn render(state: &AppState, frame: &mut Frame, area: Rect, small: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let bar = Style::default().bg(color(Token::Surface));
    let mut segs: Vec<Segment> = Vec::new();
    let demo_style = Style::default()
        .fg(color(Token::Background))
        .bg(color(Token::Info))
        .add_modifier(Modifier::BOLD);
    segs.push(seg(
        if state.demo { " DEMO " } else { " qorvi " },
        demo_style,
        0,
    ));
    if state.prefix_pending {
        // Resaltado con acento (no aviso): es un estado esperado, sin timeout.
        segs.push(seg(
            " Ctrl+Space… ",
            Style::default()
                .fg(color(Token::Background))
                .bg(color(Token::Accent))
                .add_modifier(Modifier::BOLD),
            0,
        ));
    }
    if let Some(op) = &state.busy {
        segs.push(seg(
            format!("{} {op}…", state.spinner_frame()),
            super::bold(Token::Accent),
            0,
        ));
    }
    if small {
        segs.push(seg("terminal pequeña", fg(Token::Warning), 2));
    }
    match state.active_task() {
        Some(task) => {
            segs.push(seg(task.name.clone(), fg(Token::Text), 0));
            segs.push(seg(task.branch.clone(), fg(Token::Muted), 5));
            segs.push(seg(
                format!(
                    "{} {}",
                    status_symbol(task.status, state.ascii),
                    status_label(task.status)
                ),
                fg(status_token(task.status)),
                1,
            ));
            segs.push(seg(
                format!("foco: {}", state.focus.label()),
                fg(Token::Text),
                4,
            ));
            segs.push(seg(format!("${:.2}", task.cost_usd), fg(Token::Muted), 6));
        }
        None => segs.push(seg("sin tareas", fg(Token::Muted), 1)),
    }
    segs.push(seg(
        format!(
            "ayuda: {}",
            shortcut_label(state.shortcut_style, state.direct_mod, "help")
        ),
        fg(Token::Muted),
        7,
    ));

    let sep_w = 3;
    let total = |s: &[Segment]| {
        s.iter().map(|x| text_width(&x.text)).sum::<usize>() + sep_w * s.len().saturating_sub(1)
    };
    let width = usize::from(area.width);
    while total(&segs) > width {
        let Some(max) = segs.iter().map(|s| s.drop_rank).max().filter(|r| *r > 0) else {
            break;
        };
        if let Some(pos) = segs.iter().rposition(|s| s.drop_rank == max) {
            segs.remove(pos);
        }
    }

    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = width;
    for (i, s) in segs.iter().enumerate() {
        if i > 0 {
            let sep = " │ ";
            if remaining < sep_w {
                break;
            }
            spans.push(Span::styled(sep, fg(Token::Border)));
            remaining -= sep_w;
        }
        let t = truncate(&s.text, remaining);
        remaining = remaining.saturating_sub(text_width(&t));
        spans.push(Span::styled(t, s.style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), area);
}
