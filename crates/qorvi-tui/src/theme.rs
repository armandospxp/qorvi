//! Tema `qorvi-dark` y símbolos de estado.

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::state::AgentStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Background,
    Surface,
    SurfaceActive,
    Border,
    Text,
    Muted,
    Accent,
    Info,
    Warning,
    Error,
}

pub fn rgb(token: Token) -> (u8, u8, u8) {
    match token {
        Token::Background => (0x10, 0x17, 0x20),
        Token::Surface => (0x19, 0x24, 0x33),
        Token::SurfaceActive => (0x28, 0x33, 0x43),
        Token::Border => (0x36, 0x44, 0x54),
        Token::Text => (0xD7, 0xE5, 0xF4),
        Token::Muted => (0x8B, 0x9E, 0xB5),
        Token::Accent => (0x8D, 0xE0, 0xB5),
        Token::Info => (0x87, 0xBF, 0xFF),
        Token::Warning => (0xF9, 0xCF, 0x75),
        Token::Error => (0xF2, 0x9C, 0x9C),
    }
}

fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR
        .get_or_init(|| std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit"))
}

pub fn color(token: Token) -> Color {
    color_for(token, truecolor())
}

pub fn color_for(token: Token, truecolor: bool) -> Color {
    let (r, g, b) = rgb(token);
    if truecolor {
        Color::Rgb(r, g, b)
    } else {
        Color::Indexed(nearest_256(r, g, b))
    }
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2) as u32;
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

fn nearest_cube_level(v: u8) -> usize {
    (0..CUBE.len())
        .min_by_key(|&i| (i32::from(CUBE[i]) - i32::from(v)).abs())
        .unwrap_or(0)
}

/// Índice xterm-256 más cercano: cubo 6×6×6 (16..=231) o rampa de grises (232..=255).
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (
        nearest_cube_level(r),
        nearest_cube_level(g),
        nearest_cube_level(b),
    );
    let cube_rgb = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;

    let avg = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let gray_i = (avg.saturating_sub(8) + 5) / 10;
    let gray_i = gray_i.min(23) as u8;
    let gray_v = 8 + 10 * gray_i;
    let gray_idx = 232 + usize::from(gray_i);

    if dist((r, g, b), (gray_v, gray_v, gray_v)) < dist((r, g, b), cube_rgb) {
        gray_idx as u8
    } else {
        cube_idx as u8
    }
}

pub fn status_symbol(status: AgentStatus, ascii: bool) -> &'static str {
    match (status, ascii) {
        (AgentStatus::Working, false) => "●",
        (AgentStatus::NeedsPermission, false) => "⏳",
        (AgentStatus::Done, false) => "✓",
        (AgentStatus::Error, false) => "✗",
        (AgentStatus::Idle, false) => "○",
        (AgentStatus::Connecting, false) => "…",
        (AgentStatus::Disconnected, false) => "·",
        (AgentStatus::Working, true) => "*",
        (AgentStatus::NeedsPermission, true) => "?",
        (AgentStatus::Done, true) => "+",
        (AgentStatus::Error, true) => "x",
        (AgentStatus::Idle, true) => "o",
        (AgentStatus::Connecting, true) => "~",
        (AgentStatus::Disconnected, true) => "-",
    }
}

pub fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Disconnected => "desconectado",
        AgentStatus::Connecting => "conectando",
        AgentStatus::Idle => "en espera",
        AgentStatus::Working => "trabajando",
        AgentStatus::NeedsPermission => "permiso",
        AgentStatus::Done => "listo",
        AgentStatus::Error => "error",
    }
}

pub fn status_token(status: AgentStatus) -> Token {
    match status {
        AgentStatus::Working => Token::Info,
        AgentStatus::NeedsPermission => Token::Warning,
        AgentStatus::Done => Token::Accent,
        AgentStatus::Error => Token::Error,
        AgentStatus::Idle | AgentStatus::Connecting | AgentStatus::Disconnected => Token::Muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_256_known_values() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        // Gris medio exacto de la rampa: 8 + 10*12 = 128.
        assert_eq!(nearest_256(128, 128, 128), 244);
    }

    #[test]
    fn tokens_degrade_without_truecolor() {
        assert_eq!(color_for(Token::Accent, true), Color::Rgb(0x8D, 0xE0, 0xB5));
        assert!(matches!(color_for(Token::Accent, false), Color::Indexed(_)));
    }

    #[test]
    fn ascii_symbols_differ() {
        assert_eq!(status_symbol(AgentStatus::Working, false), "●");
        assert_eq!(status_symbol(AgentStatus::Working, true), "*");
    }
}
