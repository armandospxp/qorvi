//! Codificación de eventos de ratón para el programa de un panel (convenciones xterm,
//! ver «Mouse Tracking» en ctlseqs).

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Convierte un evento de ratón de crossterm a los bytes que espera un programa que pidió
/// ratón. `column` y `row` son relativos al panel y de base 0.
///
/// Devuelve `None` si el programa no pidió ese tipo de evento (según `mode`) o si las
/// coordenadas no caben en la codificación (X10 admite hasta la celda 223; UTF-8, 2015).
///
/// - `Press`: solo pulsaciones y rueda (sin modificadores, como el modo X10 original).
/// - `PressRelease`: además soltar.
/// - `ButtonMotion`: además arrastre con botón pulsado.
/// - `AnyMotion`: además movimiento sin botón.
pub fn encode_mouse(
    ev: &MouseEvent,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    use MouseProtocolMode as M;
    // (código base del botón, es soltar)
    let (base, release) = match ev.kind {
        MouseEventKind::Down(b) => (button_code(b), false),
        MouseEventKind::Up(b) => (button_code(b), true),
        MouseEventKind::Drag(b) => (button_code(b) + 32, false),
        MouseEventKind::Moved => (3 + 32, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };
    let allowed = match ev.kind {
        MouseEventKind::Down(_)
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => mode != M::None,
        MouseEventKind::Up(_) => matches!(mode, M::PressRelease | M::ButtonMotion | M::AnyMotion),
        MouseEventKind::Drag(_) => matches!(mode, M::ButtonMotion | M::AnyMotion),
        MouseEventKind::Moved => mode == M::AnyMotion,
    };
    if !allowed {
        return None;
    }

    let mut code: u8 = base;
    if mode != M::Press {
        if ev.modifiers.contains(KeyModifiers::SHIFT) {
            code += 4;
        }
        if ev.modifiers.contains(KeyModifiers::ALT) {
            code += 8;
        }
        if ev.modifiers.contains(KeyModifiers::CONTROL) {
            code += 16;
        }
    }
    let (x, y) = (u32::from(ev.column) + 1, u32::from(ev.row) + 1);

    match encoding {
        MouseProtocolEncoding::Sgr => {
            let fin = if release { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{fin}").into_bytes())
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            // Fuera de SGR soltar no dice qué botón: siempre 3.
            let code = if release { (code & !3) | 3 } else { code };
            let mut out = b"\x1b[M".to_vec();
            if encoding == MouseProtocolEncoding::Default {
                if x + 32 > 255 || y + 32 > 255 {
                    return None;
                }
                out.extend([code + 32, x as u8 + 32, y as u8 + 32]);
            } else {
                if x + 32 > 2047 || y + 32 > 2047 {
                    return None;
                }
                for v in [u32::from(code) + 32, x + 32, y + 32] {
                    let mut buf = [0u8; 4];
                    out.extend(char::from_u32(v)?.encode_utf8(&mut buf).as_bytes());
                }
            }
            Some(out)
        }
    }
}

fn button_code(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseProtocolEncoding as E;
    use MouseProtocolMode as M;

    fn ev(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }
    }

    fn down(column: u16, row: u16) -> MouseEvent {
        ev(
            MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            KeyModifiers::NONE,
        )
    }

    #[test]
    fn modo_none_no_envia_nada() {
        for e in [E::Default, E::Utf8, E::Sgr] {
            assert_eq!(encode_mouse(&down(0, 0), M::None, e), None);
        }
    }

    #[test]
    fn clic_en_cada_codificacion() {
        assert_eq!(
            encode_mouse(&down(4, 9), M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<0;5;10M"
        );
        assert_eq!(
            encode_mouse(&down(4, 9), M::PressRelease, E::Default).unwrap(),
            [0x1b, b'[', b'M', 32, 37, 42]
        );
        assert_eq!(
            encode_mouse(&down(4, 9), M::PressRelease, E::Utf8).unwrap(),
            [0x1b, b'[', b'M', 32, 37, 42]
        );
    }

    #[test]
    fn botones_medio_y_derecho() {
        let m = ev(
            MouseEventKind::Down(MouseButton::Middle),
            0,
            0,
            KeyModifiers::NONE,
        );
        let r = ev(
            MouseEventKind::Down(MouseButton::Right),
            0,
            0,
            KeyModifiers::NONE,
        );
        assert_eq!(
            encode_mouse(&m, M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<1;1;1M"
        );
        assert_eq!(
            encode_mouse(&r, M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<2;1;1M"
        );
    }

    #[test]
    fn soltar_en_sgr_y_en_x10() {
        let up = ev(
            MouseEventKind::Up(MouseButton::Right),
            1,
            2,
            KeyModifiers::NONE,
        );
        assert_eq!(
            encode_mouse(&up, M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<2;2;3m"
        );
        // Sin SGR no se sabe qué botón se soltó: código 3.
        assert_eq!(
            encode_mouse(&up, M::PressRelease, E::Default).unwrap(),
            [0x1b, b'[', b'M', 35, 34, 35]
        );
        // El modo Press (X10) no informa de soltar.
        assert_eq!(encode_mouse(&up, M::Press, E::Sgr), None);
    }

    #[test]
    fn modificadores() {
        let e = ev(
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
            KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
        );
        assert_eq!(
            encode_mouse(&e, M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<28;1;1M"
        );
        let c = ev(
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
            KeyModifiers::CONTROL,
        );
        assert_eq!(
            encode_mouse(&c, M::AnyMotion, E::Sgr).unwrap(),
            b"\x1b[<16;1;1M"
        );
        // X10 puro no lleva modificadores.
        assert_eq!(encode_mouse(&e, M::Press, E::Sgr).unwrap(), b"\x1b[<0;1;1M");
    }

    #[test]
    fn rueda() {
        let kinds = [
            (MouseEventKind::ScrollUp, 64),
            (MouseEventKind::ScrollDown, 65),
            (MouseEventKind::ScrollLeft, 66),
            (MouseEventKind::ScrollRight, 67),
        ];
        for (k, code) in kinds {
            let e = ev(k, 2, 3, KeyModifiers::NONE);
            assert_eq!(
                encode_mouse(&e, M::Press, E::Sgr).unwrap(),
                format!("\x1b[<{code};3;4M").into_bytes()
            );
        }
        let e = ev(MouseEventKind::ScrollUp, 0, 0, KeyModifiers::NONE);
        assert_eq!(
            encode_mouse(&e, M::PressRelease, E::Default).unwrap(),
            [0x1b, b'[', b'M', 96, 33, 33]
        );
    }

    #[test]
    fn arrastre_y_movimiento_segun_modo() {
        let drag = ev(
            MouseEventKind::Drag(MouseButton::Left),
            0,
            0,
            KeyModifiers::NONE,
        );
        let moved = ev(MouseEventKind::Moved, 0, 0, KeyModifiers::NONE);
        assert_eq!(encode_mouse(&drag, M::PressRelease, E::Sgr), None);
        assert_eq!(
            encode_mouse(&drag, M::ButtonMotion, E::Sgr).unwrap(),
            b"\x1b[<32;1;1M"
        );
        assert_eq!(encode_mouse(&moved, M::ButtonMotion, E::Sgr), None);
        assert_eq!(
            encode_mouse(&drag, M::AnyMotion, E::Sgr).unwrap(),
            b"\x1b[<32;1;1M"
        );
        assert_eq!(
            encode_mouse(&moved, M::AnyMotion, E::Sgr).unwrap(),
            b"\x1b[<35;1;1M"
        );
        assert_eq!(
            encode_mouse(&moved, M::AnyMotion, E::Default).unwrap(),
            [0x1b, b'[', b'M', 67, 33, 33]
        );
    }

    #[test]
    fn limites_de_x10_y_utf8() {
        // Celda 222 (base 0) = 223 en base 1: el máximo de X10.
        assert_eq!(
            encode_mouse(&down(222, 0), M::PressRelease, E::Default).unwrap(),
            [0x1b, b'[', b'M', 32, 255, 33]
        );
        assert_eq!(
            encode_mouse(&down(223, 0), M::PressRelease, E::Default),
            None
        );
        assert_eq!(
            encode_mouse(&down(0, 223), M::PressRelease, E::Default),
            None
        );
        // UTF-8: hasta 2015 en base 1.
        let b = encode_mouse(&down(2014, 0), M::PressRelease, E::Utf8).unwrap();
        assert_eq!(b, "\x1b[M \u{7ff}!".as_bytes());
        assert_eq!(encode_mouse(&down(2015, 0), M::PressRelease, E::Utf8), None);
        // Valores > 127 ocupan 2 bytes en UTF-8.
        let b = encode_mouse(&down(200, 0), M::PressRelease, E::Utf8).unwrap();
        assert_eq!(b, "\x1b[M \u{e9}!".as_bytes());
        // SGR no tiene límite.
        assert_eq!(
            encode_mouse(&down(5000, 6000), M::PressRelease, E::Sgr).unwrap(),
            b"\x1b[<0;5001;6001M"
        );
    }
}
