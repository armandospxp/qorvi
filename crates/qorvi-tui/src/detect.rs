//! Detección de capacidades del teclado de la terminal y diagnóstico (`qorvi doctor keys`).

use crate::state::ShortcutStyle;

/// Elige la forma de atajo. `env` es el valor de `QORVI_KEYS` (`direct`, `prefix` o
/// `auto`); `enhancement` es el resultado de `supports_keyboard_enhancement` (`None` si
/// la consulta falló).
pub fn resolve_style(
    env: Option<&str>,
    is_windows: bool,
    enhancement: Option<bool>,
) -> ShortcutStyle {
    match env.map(str::to_ascii_lowercase).as_deref() {
        Some("direct") => ShortcutStyle::Direct,
        Some("prefix") => ShortcutStyle::Prefix,
        // `auto`, vacío o un valor desconocido.
        _ => {
            if is_windows || enhancement == Some(true) {
                ShortcutStyle::Direct
            } else {
                ShortcutStyle::Prefix
            }
        }
    }
}

/// ¿Fuerza `QORVI_KEYS` un estilo? (`auto` y los valores desconocidos, no).
fn forced(env: Option<&str>) -> bool {
    matches!(
        env.map(str::to_ascii_lowercase).as_deref(),
        Some("direct" | "prefix")
    )
}

/// Aviso de arranque: solo en macOS, cuando la detección automática cayó en el prefijo.
pub fn startup_hint(
    env: Option<&str>,
    is_macos: bool,
    style: ShortcutStyle,
) -> Option<&'static str> {
    (is_macos && !forced(env) && style == ShortcutStyle::Prefix)
        .then_some("Tu terminal no envía Cmd: usa Ctrl+Space o el ratón (qorvi doctor keys)")
}

/// Informe de `qorvi doctor keys`. Función pura: el binario aporta el entorno.
pub fn doctor_keys_report(
    term_program: Option<&str>,
    is_macos: bool,
    is_windows: bool,
    enhancement: Option<bool>,
) -> String {
    let modifier = if is_macos { "Cmd" } else { "Ctrl" };
    let style = resolve_style(None, is_windows, enhancement);
    let mut out = vec![
        "qorvi doctor keys".to_string(),
        String::new(),
        format!(
            "Terminal detectada: {}",
            term_program.unwrap_or("desconocida")
        ),
        format!(
            "Protocolo de teclado kitty: {}",
            match (is_windows, enhancement) {
                (true, _) => "no aplica (Windows entrega todas las teclas)",
                (_, Some(true)) => "sí",
                (_, Some(false)) => "no",
                (_, None) => "no se pudo comprobar",
            }
        ),
    ];
    out.push(match style {
        ShortcutStyle::Direct => {
            format!("Atajos directos ({modifier}+...): funcionarán")
        }
        ShortcutStyle::Prefix => {
            format!("Atajos directos ({modifier}+...): no llegan a qorvi en esta terminal")
        }
    });
    out.push(format!(
        "Estilo automático: {}",
        match style {
            ShortcutStyle::Direct => "directo (la ayuda y las pistas muestran los atajos directos)",
            ShortcutStyle::Prefix => "prefijo (Ctrl+Space + tecla; el ratón también funciona)",
        }
    ));
    out.push("Fuerza otro estilo con QORVI_KEYS=direct|prefix|auto.".to_string());
    out.push(String::new());

    match term_program {
        Some("ghostty") if is_macos => {
            out.push(
                "Ghostty usa algunos Cmd para sí. Para liberarlos, añade esto a \
                 ~/.config/ghostty/config y reinicia Ghostty:"
                    .to_string(),
            );
            out.push(String::new());
            for n in 1..=8 {
                out.push(format!("keybind = super+digit_{n}=unbind"));
            }
            for n in 1..=9 {
                out.push(format!("keybind = super+{n}=unbind"));
            }
            for dir in ["up", "down", "left", "right"] {
                out.push(format!("keybind = super+alt+arrow_{dir}=unbind"));
            }
            out.push("keybind = super+j=unbind".to_string());
            out.push(String::new());
            out.push(
                "Después, para cambiar de pestaña de Ghostty usa Cmd+Shift+[ y Cmd+Shift+]."
                    .to_string(),
            );
        }
        Some("ghostty") => {
            out.push("Ghostty: los atajos usan Ctrl; no hace falta liberar ningún Cmd.".to_string())
        }
        Some("Apple_Terminal") => out.push(
            "Terminal.app no envía Cmd a las aplicaciones: usa Ctrl+Space o el ratón, \
             o cambia a Ghostty, iTerm2, kitty o WezTerm."
                .to_string(),
        ),
        Some("vscode") => {
            out.push("La terminal de VS Code captura Cmd: usa Ctrl+Space o el ratón.".to_string())
        }
        Some("iTerm.app") => {
            out.push(
                "iTerm2: si la detección falla, activa \"Report keys using CSI u\" (protocolo \
                 kitty) en Profiles → Keys."
                    .to_string(),
            );
            out.push("Sin verificar: no se ha probado el protocolo en iTerm2.".to_string());
        }
        _ => out.push(
            "Terminal sin guía específica. Si los atajos directos no responden, usa \
             Ctrl+Space o el ratón."
                .to_string(),
        ),
    }
    let mut report = out.join("\n");
    report.push('\n');
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_style_truth_table() {
        use ShortcutStyle::{Direct, Prefix};
        // (env, windows, enhancement, esperado)
        let cases: [(Option<&str>, bool, Option<bool>, ShortcutStyle); 14] = [
            (None, false, Some(true), Direct),
            (None, false, Some(false), Prefix),
            (None, false, None, Prefix),
            (None, true, Some(false), Direct),
            (None, true, None, Direct),
            (Some("auto"), false, Some(true), Direct),
            (Some("auto"), false, Some(false), Prefix),
            (Some("direct"), false, Some(false), Direct),
            (Some("DIRECT"), false, None, Direct),
            (Some("prefix"), false, Some(true), Prefix),
            (Some("prefix"), true, Some(true), Prefix),
            (Some("raro"), false, Some(true), Direct),
            (Some("raro"), false, Some(false), Prefix),
            (Some(""), true, None, Direct),
        ];
        for (env, win, enh, want) in cases {
            assert_eq!(resolve_style(env, win, enh), want, "{env:?} {win} {enh:?}");
        }
    }

    #[test]
    fn startup_hint_only_for_auto_prefix_on_macos() {
        use ShortcutStyle::{Direct, Prefix};
        assert!(startup_hint(None, true, Prefix).is_some());
        assert!(startup_hint(Some("auto"), true, Prefix).is_some());
        assert!(startup_hint(Some("prefix"), true, Prefix).is_none());
        assert!(startup_hint(None, true, Direct).is_none());
        assert!(startup_hint(None, false, Prefix).is_none());
    }

    #[test]
    fn report_ghostty_lists_unbind_lines() {
        let r = doctor_keys_report(Some("ghostty"), true, false, Some(true));
        for line in [
            "keybind = super+digit_1=unbind",
            "keybind = super+digit_8=unbind",
            "keybind = super+1=unbind",
            "keybind = super+9=unbind",
            "keybind = super+alt+arrow_up=unbind",
            "keybind = super+alt+arrow_down=unbind",
            "keybind = super+alt+arrow_left=unbind",
            "keybind = super+alt+arrow_right=unbind",
            "keybind = super+j=unbind",
            "Cmd+Shift+[",
            "~/.config/ghostty/config",
            "Atajos directos (Cmd+...): funcionarán",
        ] {
            assert!(r.contains(line), "falta {line:?} en:\n{r}");
        }
        assert!(!r.contains("super+digit_9"));
    }

    #[test]
    fn report_per_terminal() {
        let apple = doctor_keys_report(Some("Apple_Terminal"), true, false, Some(false));
        assert!(apple.contains("Terminal.app no envía Cmd"));
        assert!(apple.contains("Estilo automático: prefijo"));
        assert!(!apple.contains("keybind"));

        let vscode = doctor_keys_report(Some("vscode"), true, false, Some(false));
        assert!(vscode.contains("VS Code captura Cmd"));

        let iterm = doctor_keys_report(Some("iTerm.app"), true, false, Some(false));
        assert!(iterm.contains("Report keys using CSI u"));
        assert!(iterm.contains("Sin verificar"));

        let unknown = doctor_keys_report(None, false, false, None);
        assert!(unknown.contains("Terminal detectada: desconocida"));
        assert!(unknown.contains("no se pudo comprobar"));
        assert!(unknown.contains("Terminal sin guía específica"));

        let win = doctor_keys_report(Some("WindowsTerminal"), false, true, None);
        assert!(win.contains("Atajos directos (Ctrl+...): funcionarán"));
        assert!(win.contains("Estilo automático: directo"));
    }
}
