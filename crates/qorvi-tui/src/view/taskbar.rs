//! Barra de tareas: `[1 APP-12 ● trabajando] [2 APP-13 ⏳ permiso] …`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{fg, text_width, truncate};
use crate::state::AppState;
use crate::theme::{color, status_label, status_symbol, status_token, Token};

pub fn render(state: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let label = |i: usize, short: bool| -> String {
        let t = &state.tasks[i];
        let sym = status_symbol(t.status, state.ascii);
        if short {
            format!("[{} {} {}]", i + 1, t.name, sym)
        } else {
            format!("[{} {} {} {}]", i + 1, t.name, sym, status_label(t.status))
        }
    };
    let n = state.tasks.len();
    let total = |short_inactive: bool| -> usize {
        (0..n)
            .map(|i| text_width(&label(i, short_inactive && i != state.active)))
            .sum::<usize>()
            + n.saturating_sub(1)
            + 1
    };
    let short_inactive = total(false) > width;

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    if n == 0 {
        spans.push(Span::styled("(sin tareas)", fg(Token::Muted)));
    }
    for i in 0..n {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let active = i == state.active;
        let style = if active {
            Style::default()
                .fg(color(status_token(state.tasks[i].status)))
                .bg(color(Token::SurfaceActive))
                .add_modifier(Modifier::BOLD)
        } else {
            fg(Token::Muted)
        };
        spans.push(Span::styled(label(i, short_inactive && !active), style));
    }
    let line = Line::from(spans);
    if line.width() > width {
        // Aun así no cabe: se muestra solo la tarea activa, cortada con `…`.
        let text = state
            .tasks
            .get(state.active)
            .map(|_| truncate(&format!(" {}", label(state.active, false)), width))
            .unwrap_or_default();
        frame.render_widget(Paragraph::new(Line::styled(text, fg(Token::Text))), area);
    } else {
        frame.render_widget(Paragraph::new(line), area);
    }
}
