//! Barra de tareas: `[1 APP-12 ● trabajando] [2 APP-13 ⏳ permiso] …`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::fg;
use crate::layout::Areas;
use crate::state::AppState;
use crate::theme::{color, status_token, Token};

pub fn render(state: &AppState, frame: &mut Frame, areas: &Areas) {
    let area = areas.taskbar;
    if area.width == 0 || area.height == 0 {
        return;
    }
    if areas.task_overflow {
        // No cabe ninguna tarea completa: solo la activa, cortada con `…`.
        let text = areas
            .task_tabs
            .first()
            .map(|t| t.label.clone())
            .unwrap_or_default();
        frame.render_widget(Paragraph::new(Line::styled(text, fg(Token::Text))), area);
        return;
    }
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    if state.tasks.is_empty() {
        spans.push(Span::styled("(sin tareas)", fg(Token::Muted)));
    }
    for (n, tab) in areas.task_tabs.iter().enumerate() {
        if n > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if tab.index == state.active {
            Style::default()
                .fg(color(status_token(state.tasks[tab.index].status)))
                .bg(color(Token::SurfaceActive))
                .add_modifier(Modifier::BOLD)
        } else {
            fg(Token::Muted)
        };
        spans.push(Span::styled(tab.label.clone(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
