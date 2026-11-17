//! Codificación de teclas y pegado para el programa de un panel (convenciones xterm).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Convierte una tecla de crossterm a los bytes que espera un programa en un PTY
/// (codificación xterm). `app_cursor` = el hijo activó DECCKM (nvim, less…).
pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let mods = key.modifiers;
    let shift = mods.contains(KeyModifiers::SHIFT);
    let alt = mods.contains(KeyModifiers::ALT);
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    // Parámetro de modificador xterm: 1 + shift + 2*alt + 4*ctrl.
    let modparam = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    // Teclas "CSI final": flechas, Home, End.
    let csi_final = match key.code {
        KeyCode::Up => Some('A'),
        KeyCode::Down => Some('B'),
        KeyCode::Right => Some('C'),
        KeyCode::Left => Some('D'),
        KeyCode::Home => Some('H'),
        KeyCode::End => Some('F'),
        _ => None,
    };
    if let Some(f) = csi_final {
        return Some(if modparam > 1 {
            format!("\x1b[1;{modparam}{f}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{f}").into_bytes()
        } else {
            format!("\x1b[{f}").into_bytes()
        });
    }

    // Teclas "CSI n ~".
    let tilde = match key.code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        KeyCode::F(5) => Some(15),
        KeyCode::F(6) => Some(17),
        KeyCode::F(7) => Some(18),
        KeyCode::F(8) => Some(19),
        KeyCode::F(9) => Some(20),
        KeyCode::F(10) => Some(21),
        KeyCode::F(11) => Some(23),
        KeyCode::F(12) => Some(24),
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(if modparam > 1 {
            format!("\x1b[{n};{modparam}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        });
    }

    let mut out = match key.code {
        KeyCode::F(n @ 1..=4) => {
            let f = ['P', 'Q', 'R', 'S'][(n - 1) as usize];
            return Some(if modparam > 1 {
                format!("\x1b[1;{modparam}{f}").into_bytes()
            } else {
                format!("\x1bO{f}").into_bytes()
            });
        }
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![c as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            ']' | '5' => vec![0x1d],
            '^' | '6' => vec![0x1e],
            '_' | '7' | '-' => vec![0x1f],
            _ => return None,
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace if ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        _ => return None,
    };
    // Alt = prefijo ESC (convención "meta sends escape").
    if alt {
        out.insert(0, 0x1b);
    }
    Some(out)
}

/// Bytes de un pegado. Con `bracketed` (el programa activó el modo 2004), el texto va
/// entre `ESC [200~` y `ESC [201~` para que el programa no lo ejecute como si se tecleara.
///
/// Los saltos de línea se envían como `\r`, como hace una terminal al pegar. Se eliminan
/// los `ESC` del texto: un `ESC [201~` pegado cerraría el modo antes de tiempo y el resto
/// del portapapeles se ejecutaría como órdenes.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text
        .replace("\r\n", "\r")
        .replace('\n', "\r")
        .replace('\x1b', "");
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_and_ctrl_chars() {
        assert_eq!(
            encode_key(k(KeyCode::Char('a'), KeyModifiers::NONE), false).unwrap(),
            b"a"
        );
        assert_eq!(
            encode_key(k(KeyCode::Char('ñ'), KeyModifiers::NONE), false).unwrap(),
            "ñ".as_bytes()
        );
        assert_eq!(
            encode_key(k(KeyCode::Char('c'), KeyModifiers::CONTROL), false).unwrap(),
            [3]
        );
        assert_eq!(
            encode_key(k(KeyCode::Char('x'), KeyModifiers::ALT), false).unwrap(),
            b"\x1bx"
        );
    }

    #[test]
    fn arrows_respect_application_cursor_and_modifiers() {
        assert_eq!(
            encode_key(k(KeyCode::Up, KeyModifiers::NONE), false).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key(k(KeyCode::Up, KeyModifiers::NONE), true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(k(KeyCode::Left, KeyModifiers::CONTROL), true).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(k(KeyCode::Delete, KeyModifiers::SHIFT), false).unwrap(),
            b"\x1b[3;2~"
        );
        assert_eq!(
            encode_key(k(KeyCode::F(1), KeyModifiers::NONE), false).unwrap(),
            b"\x1bOP"
        );
    }

    #[test]
    fn pegado() {
        assert_eq!(encode_paste("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(encode_paste("ls", true), b"\x1b[200~ls\x1b[201~");
        // Un cierre de pegado dentro del texto no escapa del modo bracketed.
        assert_eq!(
            encode_paste("x\x1b[201~rm -rf ~\n", true),
            b"\x1b[200~x[201~rm -rf ~\r\x1b[201~"
        );
    }
}
