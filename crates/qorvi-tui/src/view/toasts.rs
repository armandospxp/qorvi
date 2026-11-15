//! Avisos en la esquina inferior derecha, encima de la barra de estado.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{fg, truncate};
use crate::state::{AppState, ToastLevel};
use crate::theme::Token;

const MAX_VISIBLE: usize = 3;
const TOAST_H: u16 = 3;
const TOAST_W: u16 = 46;

pub fn render(state: &AppState, frame: &mut Frame, area: Rect) {
    let w = TOAST_W.min(area.width);
    if w < 6 {
        return;
    }
    for (i, toast) in state.toasts.iter().rev().take(MAX_VISIBLE).enumerate() {
        let used = TOAST_H * (i as u16 + 1);
        if area.height < used {
            break;
        }
        let rect = Rect::new(area.right() - w, area.bottom() - used, w, TOAST_H);
        let (token, tag) = match toast.level {
            ToastLevel::Info => (Token::Info, "info"),
            ToastLevel::Warning => (Token::Warning, "aviso"),
            ToastLevel::Error => (Token::Error, "error"),
        };
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(fg(token));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let text = truncate(&format!("[{tag}] {}", toast.text), usize::from(inner.width));
        frame.render_widget(Paragraph::new(Line::styled(text, fg(token))), inner);
    }
}
