//! Vista: pura, nunca muta el estado.

pub mod chat;
pub mod explorer;
pub mod modals;
pub mod panes;
pub mod review;
pub mod statusbar;
pub mod taskbar;
pub mod term;
pub mod toasts;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use crate::layout;
use crate::state::{AppState, Modal, Pane};
use crate::theme::{color, Token};

pub const MIN_COLS: u16 = 40;
pub const MIN_ROWS: u16 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    TooSmall,
    Compact,
    /// 90–119 columnas: terminal minimizada y explorador oculto por defecto.
    Medium,
    Full,
}

pub fn tier(width: u16, height: u16) -> Tier {
    if width < MIN_COLS || height < MIN_ROWS {
        Tier::TooSmall
    } else if width >= 120 && height >= 35 {
        Tier::Full
    } else if width >= 90 && height >= 20 {
        Tier::Medium
    } else {
        Tier::Compact
    }
}

pub fn is_compact(width: u16, height: u16) -> bool {
    tier(width, height) == Tier::Compact
}

/// Aviso "terminal pequeña" para la barra de estado (< 70×20).
pub fn is_small(width: u16, height: u16) -> bool {
    width < 70 || height < 20
}

pub fn fg(token: Token) -> Style {
    Style::default().fg(color(token))
}

pub fn bold(token: Token) -> Style {
    fg(token).add_modifier(Modifier::BOLD)
}

pub fn text_width(s: &str) -> usize {
    Line::from(s).width()
}

fn char_width(c: char) -> usize {
    Line::from(c.to_string()).width()
}

/// Corta `s` a `max` columnas añadiendo `…` si hace falta.
pub fn truncate(s: &str, max: usize) -> String {
    if text_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if w + cw > max - 1 {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Conserva el final de `s` (para campos de texto con el cursor al final).
pub fn tail(s: &str, max: usize) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut w = 0;
    for c in s.chars().rev() {
        let cw = char_width(c);
        if w + cw > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.into_iter().rev().collect()
}

/// Ajuste de línea por palabras; las palabras más largas que `width` se parten.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut cur = String::new();
        let mut cw = 0;
        for word in line.split(' ') {
            let ww = text_width(word);
            let sep = usize::from(!cur.is_empty());
            if cw + sep + ww <= width {
                if sep == 1 {
                    cur.push(' ');
                }
                cur.push_str(word);
                cw += sep + ww;
                continue;
            }
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cw = 0;
            }
            for ch in word.chars() {
                let w = char_width(ch);
                if cw + w > width && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    cw = 0;
                }
                cur.push(ch);
                cw += w;
            }
        }
        out.push(cur);
    }
    out
}

/// Rectángulo centrado de como máximo `w`×`h`, siempre dentro de `area`.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// Dibuja la interfaz (demo: sin paneles reales).
pub fn view(state: &AppState, frame: &mut Frame) {
    view_with(state, frame, &term::NoScreens);
}

/// Dibuja la interfaz con las pantallas de los paneles reales.
pub fn view_with(state: &AppState, frame: &mut Frame, screens: &dyn term::Screens) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(
            Style::default()
                .bg(color(Token::Background))
                .fg(color(Token::Text)),
        ),
        area,
    );
    let tier = tier(area.width, area.height);
    if tier == Tier::TooSmall {
        render_too_small(frame, area);
        return;
    }

    let areas = layout::compute(area, state);
    taskbar::render(state, frame, &areas);
    render_body(state, frame, &areas, screens);
    statusbar::render(
        state,
        frame,
        areas.statusbar,
        is_small(area.width, area.height),
    );
    let body = areas.body;
    if let Modal::Review { files, selected } = &state.modal {
        review::render(state, frame, body, files, *selected);
    } else if state.modal != Modal::None {
        modals::render(state, frame, body);
    }
    toasts::render(state, frame, body);
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let text = "Terminal demasiado pequeña (mín. 40×10)";
    let w = usize::from(area.width.max(1));
    let rows = text_width(text)
        .div_ceil(w)
        .clamp(1, usize::from(area.height.max(1))) as u16;
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(rows),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        mid,
    );
}

fn render_body(
    state: &AppState,
    frame: &mut Frame,
    areas: &layout::Areas,
    screens: &dyn term::Screens,
) {
    if let Some(tabs) = areas.tabs_bar {
        panes::render_tabs(state, frame, tabs);
    }
    for (pane, rect) in areas.panes() {
        if pane == Pane::Terminal && areas.terminal_minimized {
            panes::render_minimized_terminal(state, frame, rect);
        } else {
            panes::render_pane(state, frame, rect, pane, screens);
        }
    }
}
