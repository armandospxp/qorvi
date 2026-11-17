//! Respuestas a las consultas que los programas hacen a "su terminal".
//!
//! `vt100` emula la pantalla pero no contesta consultas como la posición del cursor (DSR)
//! o el tipo de terminal (DA1). Sin respuesta, nvim, fish y otras TUIs esperan un timeout
//! al arrancar. [`Responder`] implementa los callbacks de `vt100` y acumula las respuestas
//! para que el panel las escriba al PTY; como el propio parser reconoce las secuencias,
//! funciona aunque una consulta llegue partida entre dos lecturas.
//!
//! Lo que no se contesta a propósito: el protocolo de teclado kitty (`CSI ? u`) y DECRQM
//! (`CSI ? Ps $ p`). Los programas que los consultan envían DA1 justo después y, al
//! recibir esa respuesta sin la otra, deducen que no están soportados.

/// Color de primer plano que se anuncia (OSC 10).
const FG: &str = "rgb:d0d0/d0d0/d0d0";
/// Color de fondo que se anuncia (OSC 11). Oscuro, como el tema de qorvi.
const BG: &str = "rgb:1c1c/1c1c/1c1c";

/// Callbacks de `vt100`: respuestas pendientes y eventos (título, campana).
#[derive(Debug, Default)]
pub struct Responder {
    replies: Vec<u8>,
    title: Option<String>,
    title_changed: bool,
    bell: bool,
}

impl Responder {
    /// Bytes que hay que escribir al PTY. Vacía la cola.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    /// Título nuevo si el programa lo cambió desde la última llamada.
    pub fn take_title_change(&mut self) -> Option<String> {
        if std::mem::take(&mut self.title_changed) {
            Some(self.title.clone().unwrap_or_default())
        } else {
            None
        }
    }

    /// Título actual de la ventana (OSC 0/2), si el programa puso alguno.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// `true` si sonó la campana desde la última llamada.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }
}

impl vt100::Callbacks for Responder {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let title = String::from_utf8_lossy(title).into_owned();
        if self.title.as_deref() != Some(&title) {
            self.title = Some(title);
            self.title_changed = true;
        }
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            // DSR: estado del dispositivo → "todo bien".
            (None, 'n', 5) => self.replies.extend_from_slice(b"\x1b[0n"),
            // DSR: posición del cursor (base 1).
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            // DA1: VT220 con colores ANSI.
            (None, 'c', 0) => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            // DA2: tipo de terminal secundario.
            (Some(b'>'), 'c', 0) => self.replies.extend_from_slice(b"\x1b[>0;10;1c"),
            // XTVERSION: nombre y versión de la terminal.
            (Some(b'>'), 'q', 0) => self.replies.extend_from_slice(
                format!("\x1bP>|qorvi {}\x1b\\", env!("CARGO_PKG_VERSION")).as_bytes(),
            ),
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        let color = match params {
            [b"10", b"?"] => Some(("10", FG)),
            [b"11", b"?"] => Some(("11", BG)),
            _ => None,
        };
        if let Some((code, value)) = color {
            self.replies
                .extend_from_slice(format!("\x1b]{code};{value}\x1b\\").as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser() -> vt100::Parser<Responder> {
        vt100::Parser::new_with_callbacks(24, 80, 0, Responder::default())
    }

    fn reply(input: &[u8]) -> Vec<u8> {
        let mut p = parser();
        p.process(input);
        p.callbacks_mut().take_replies()
    }

    #[test]
    fn consultas_comunes() {
        assert_eq!(reply(b"\x1b[5n"), b"\x1b[0n");
        assert_eq!(reply(b"\x1b[c"), b"\x1b[?62;22c");
        assert_eq!(reply(b"\x1b[0c"), b"\x1b[?62;22c");
        assert_eq!(reply(b"\x1b[>c"), b"\x1b[>0;10;1c");
        assert!(reply(b"\x1b[>q").starts_with(b"\x1bP>|qorvi "));
        assert_eq!(reply(b"\x1b]11;?\x07"), b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\");
        assert_eq!(
            reply(b"\x1b]10;?\x1b\\"),
            b"\x1b]10;rgb:d0d0/d0d0/d0d0\x1b\\"
        );
    }

    #[test]
    fn posicion_del_cursor_en_el_momento_de_la_consulta() {
        // Mueve el cursor a fila 5, columna 10 (base 1), consulta, y sigue escribiendo:
        // la respuesta debe reflejar la posición en el momento de la consulta.
        assert_eq!(reply(b"\x1b[5;10H\x1b[6nmas texto"), b"\x1b[5;10R");
    }

    #[test]
    fn kitty_y_decrqm_no_se_contestan() {
        assert_eq!(reply(b"\x1b[?u\x1b[c"), b"\x1b[?62;22c");
        assert!(reply(b"\x1b[?2026$p").is_empty());
    }

    #[test]
    fn consulta_partida_entre_lecturas() {
        let mut p = parser();
        p.process(b"hola\x1b[c\x1b]11");
        assert_eq!(p.callbacks_mut().take_replies(), b"\x1b[?62;22c");
        p.process(b";?\x07texto");
        assert_eq!(
            p.callbacks_mut().take_replies(),
            b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\"
        );
        // Nada se contesta dos veces.
        assert!(p.callbacks_mut().take_replies().is_empty());
    }

    #[test]
    fn titulo_y_campana() {
        let mut p = parser();
        p.process(b"\x1b]2;compilando\x07");
        assert_eq!(
            p.callbacks_mut().take_title_change().as_deref(),
            Some("compilando")
        );
        assert_eq!(p.callbacks_mut().take_title_change(), None);
        // Mismo título otra vez: no es un cambio.
        p.process(b"\x1b]0;compilando\x07");
        assert_eq!(p.callbacks_mut().take_title_change(), None);
        assert_eq!(p.callbacks().title(), Some("compilando"));

        assert!(!p.callbacks_mut().take_bell());
        p.process(b"\x07");
        assert!(p.callbacks_mut().take_bell());
        assert!(!p.callbacks_mut().take_bell());
    }
}
