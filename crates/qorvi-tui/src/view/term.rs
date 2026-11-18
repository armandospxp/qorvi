//! Dibujo de la pantalla emulada (vt100) de un panel de terminal.

use qorvi_pty::vt100;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::state::Pane;
use crate::theme::{color, Token};

/// Fuente de las pantallas de los paneles reales. La demo no tiene ninguna.
pub trait Screens {
    /// Dibuja el panel `pane` de la tarea `task` en `area`. Devuelve `None` si ese panel
    /// no tiene programa (se dibuja un aviso en su lugar) y, si lo tiene, la posición
    /// del cursor cuando debe verse.
    fn draw(
        &self,
        task: &str,
        pane: Pane,
        area: Rect,
        buf: &mut Buffer,
    ) -> Option<Option<Position>>;
}

/// Sin pantallas (demo y tests).
pub struct NoScreens;

impl Screens for NoScreens {
    fn draw(&self, _: &str, _: Pane, _: Rect, _: &mut Buffer) -> Option<Option<Position>> {
        None
    }
}

fn to_color(c: vt100::Color, default: Token) -> Color {
    match c {
        vt100::Color::Default => color(default),
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Copia `screen` en `area` del buffer. Devuelve el cursor si `cursor` es `true`, el
/// programa no lo oculta y cae dentro del área.
pub fn draw_screen(
    screen: &vt100::Screen,
    area: Rect,
    buf: &mut Buffer,
    cursor: bool,
) -> Option<Position> {
    let (rows, cols) = screen.size();
    for row in 0..rows.min(area.height) {
        for col in 0..cols.min(area.width) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let pos = Position::new(area.x + col, area.y + row);
            let Some(out) = buf.cell_mut(pos) else {
                continue;
            };
            if cell.is_wide_continuation() {
                // La celda anterior ocupa dos columnas: esta no se pinta.
                out.set_diff_option(CellDiffOption::Skip);
                continue;
            }
            let (mut fg, mut bg) = (
                to_color(cell.fgcolor(), Token::Text),
                to_color(cell.bgcolor(), Token::Background),
            );
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut m = Modifier::empty();
            if cell.bold() {
                m |= Modifier::BOLD;
            }
            if cell.dim() {
                m |= Modifier::DIM;
            }
            if cell.italic() {
                m |= Modifier::ITALIC;
            }
            if cell.underline() {
                m |= Modifier::UNDERLINED;
            }
            let text = cell.contents();
            out.set_diff_option(CellDiffOption::None);
            out.set_symbol(if text.is_empty() { " " } else { text });
            out.set_style(Style::default().fg(fg).bg(bg).add_modifier(m));
        }
    }
    if !cursor || screen.hide_cursor() {
        return None;
    }
    let (row, col) = screen.cursor_position();
    (row < area.height && col < area.width).then(|| Position::new(area.x + col, area.y + row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copia_texto_estilos_y_cursor() {
        let mut p = vt100::Parser::new(3, 10, 0);
        p.process(b"hola \x1b[1;31mrojo\x1b[0m\r\n\xe4\xbd\xa0x");
        let area = Rect::new(2, 1, 10, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 5));
        let cur = draw_screen(p.screen(), area, &mut buf, true);
        assert_eq!(buf[(2, 1)].symbol(), "h");
        assert_eq!(buf[(7, 1)].symbol(), "r");
        assert_eq!(buf[(7, 1)].fg, Color::Indexed(1));
        assert!(buf[(7, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(2, 2)].symbol(), "你");
        assert_eq!(buf[(4, 2)].symbol(), "x");
        assert_eq!(cur, Some(Position::new(5, 2)));
        // Cursor oculto por el programa o sin foco: no se devuelve.
        p.process(b"\x1b[?25l");
        assert_eq!(draw_screen(p.screen(), area, &mut buf, true), None);
    }

    #[test]
    fn area_menor_que_la_pantalla_no_desborda() {
        let mut p = vt100::Parser::new(5, 20, 0);
        p.process(b"0123456789abcdefghij");
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        draw_screen(p.screen(), Rect::new(0, 0, 4, 2), &mut buf, true);
        assert_eq!(buf[(3, 0)].symbol(), "3");
    }
}
