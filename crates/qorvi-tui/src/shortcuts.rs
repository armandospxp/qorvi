//! Tabla única de atajos globales: la leen el mapa de teclas, la ayuda, la barra de
//! estado y todas las pistas, para que ningún texto se duplique.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::action::Action;
use crate::state::ShortcutStyle;

/// Tecla de un atajo directo (sin el modificador).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboKey {
    /// Letra; se acepta también en mayúscula.
    Letter(char),
    /// Dígitos `1..9`: el atajo lleva el índice de la tarea.
    Digits,
    Arrow(KeyCode),
    F(u8),
}

/// Atajo directo: modificador (`Cmd` o `Ctrl`, según el sistema), `Alt` opcional y tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub modifier: bool,
    pub alt: bool,
    pub key: ComboKey,
}

const fn combo(modifier: bool, alt: bool, key: ComboKey) -> Option<Combo> {
    Some(Combo { modifier, alt, key })
}

#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    pub id: &'static str,
    /// Tecla tras el prefijo, tal como se muestra.
    pub prefix: &'static str,
    /// Caracteres que la activan tras el prefijo o con `Alt+`; su posición es el índice
    /// que recibe `build`.
    pub prefix_keys: &'static str,
    pub direct: Option<Combo>,
    pub desc: &'static str,
    pub build: fn(usize) -> Action,
}

pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: "focus_left",
        prefix: "h",
        prefix_keys: "h",
        direct: combo(true, true, ComboKey::Arrow(KeyCode::Left)),
        desc: "foco al panel de la izquierda",
        build: |_| Action::FocusLeft,
    },
    Shortcut {
        id: "focus_down",
        prefix: "j",
        prefix_keys: "j",
        direct: combo(true, true, ComboKey::Arrow(KeyCode::Down)),
        desc: "foco al panel de abajo",
        build: |_| Action::FocusDown,
    },
    Shortcut {
        id: "focus_up",
        prefix: "k",
        prefix_keys: "k",
        direct: combo(true, true, ComboKey::Arrow(KeyCode::Up)),
        desc: "foco al panel de arriba",
        build: |_| Action::FocusUp,
    },
    Shortcut {
        id: "focus_right",
        prefix: "l",
        prefix_keys: "l",
        direct: combo(true, true, ComboKey::Arrow(KeyCode::Right)),
        desc: "foco al panel de la derecha",
        build: |_| Action::FocusRight,
    },
    Shortcut {
        id: "select_task",
        prefix: "1..9",
        prefix_keys: "123456789",
        direct: combo(true, false, ComboKey::Digits),
        desc: "ir a la tarea N",
        build: Action::SelectTask,
    },
    Shortcut {
        id: "new_task",
        prefix: "n",
        prefix_keys: "n",
        direct: combo(true, true, ComboKey::Letter('n')),
        desc: "nueva tarea",
        build: |_| Action::NewTask,
    },
    Shortcut {
        id: "review",
        prefix: "d",
        prefix_keys: "d",
        direct: combo(true, true, ComboKey::Letter('r')),
        desc: "review / diff",
        build: |_| Action::OpenReview,
    },
    Shortcut {
        id: "zoom",
        prefix: "z",
        prefix_keys: "z",
        direct: None,
        desc: "zoom del panel",
        build: |_| Action::ToggleZoom,
    },
    Shortcut {
        id: "toggle_terminal",
        prefix: "t",
        prefix_keys: "t",
        direct: combo(true, false, ComboKey::Letter('j')),
        desc: "mostrar/ocultar la terminal",
        build: |_| Action::ToggleTerminal,
    },
    Shortcut {
        id: "toggle_explorer",
        prefix: "e",
        prefix_keys: "e",
        direct: combo(true, false, ComboKey::Letter('b')),
        desc: "mostrar/ocultar el explorador",
        build: |_| Action::ToggleExplorer,
    },
    Shortcut {
        id: "last_toast",
        prefix: "m",
        prefix_keys: "m",
        direct: None,
        desc: "ver de nuevo el último aviso",
        build: |_| Action::ShowLastToast,
    },
    Shortcut {
        id: "quit",
        prefix: "q",
        prefix_keys: "q",
        direct: combo(true, true, ComboKey::Letter('q')),
        desc: "salir",
        build: |_| Action::Quit,
    },
    Shortcut {
        id: "help",
        prefix: "?",
        prefix_keys: "?",
        direct: combo(false, false, ComboKey::F(1)),
        desc: "esta ayuda",
        build: |_| Action::Help,
    },
];

pub fn find(id: &str) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|s| s.id == id)
}

/// Acción de un atajo tras el prefijo o con `Alt+` (la misma tecla en ambos casos).
pub fn prefix_action(c: char) -> Option<Action> {
    SHORTCUTS.iter().find_map(|s| {
        s.prefix_keys
            .chars()
            .position(|k| k == c)
            .map(|i| (s.build)(i))
    })
}

/// Nombre del modificador directo: `Cmd` si es `SUPER`, `Ctrl` en cualquier otro caso.
pub fn modifier_name(direct_mod: KeyModifiers) -> &'static str {
    if direct_mod.contains(KeyModifiers::SUPER) {
        "Cmd"
    } else {
        "Ctrl"
    }
}

/// Texto del atajo directo, por ejemplo `Cmd+Alt+R` o `F1`.
pub fn direct_label(direct_mod: KeyModifiers, combo: Combo) -> String {
    let mut s = String::new();
    if combo.modifier {
        s.push_str(modifier_name(direct_mod));
        s.push('+');
    }
    if combo.alt {
        s.push_str("Alt+");
    }
    match combo.key {
        ComboKey::Letter(c) => s.push(c.to_ascii_uppercase()),
        ComboKey::Digits => s.push_str("1..9"),
        ComboKey::Arrow(KeyCode::Left) => s.push('←'),
        ComboKey::Arrow(KeyCode::Down) => s.push('↓'),
        ComboKey::Arrow(KeyCode::Up) => s.push('↑'),
        ComboKey::Arrow(_) => s.push('→'),
        ComboKey::F(n) => s.push_str(&format!("F{n}")),
    }
    s
}

/// Forma con prefijo: `Ctrl+Space d`.
pub fn prefix_label(shortcut: &Shortcut) -> String {
    format!("Ctrl+Space {}", shortcut.prefix)
}

/// Texto del atajo `action_id` en la forma que se muestra. Las acciones sin atajo
/// directo muestran siempre el prefijo.
pub fn shortcut_label(style: ShortcutStyle, direct_mod: KeyModifiers, action_id: &str) -> String {
    let Some(shortcut) = find(action_id) else {
        debug_assert!(false, "atajo desconocido: {action_id}");
        return String::new();
    };
    match (style, shortcut.direct) {
        (ShortcutStyle::Direct, Some(c)) => direct_label(direct_mod, c),
        _ => prefix_label(shortcut),
    }
}

/// ¿Coincide la tecla con el atajo directo? Devuelve el índice (la tarea, con dígitos).
pub fn match_combo(
    combo: Combo,
    direct_mod: KeyModifiers,
    key: &crossterm::event::KeyEvent,
) -> Option<usize> {
    let mut want = KeyModifiers::NONE;
    if combo.modifier {
        want |= direct_mod;
    }
    if combo.alt {
        want |= KeyModifiers::ALT;
    }
    // Shift solo se ignora en letras: pueden llegar en mayúscula (Caps Lock, Shift).
    let mut got = key.modifiers;
    if matches!(combo.key, ComboKey::Letter(_)) {
        got -= KeyModifiers::SHIFT;
    }
    if got != want {
        return None;
    }
    match (combo.key, key.code) {
        (ComboKey::Letter(l), KeyCode::Char(c)) if c.eq_ignore_ascii_case(&l) => Some(0),
        (ComboKey::Digits, KeyCode::Char(c @ '1'..='9')) => Some(c as usize - '1' as usize),
        (ComboKey::Arrow(a), code) if a == code => Some(0),
        (ComboKey::F(n), KeyCode::F(m)) if n == m => Some(0),
        _ => None,
    }
}

/// Acción del atajo directo que corresponde a la tecla, si hay.
pub fn direct_action(direct_mod: KeyModifiers, key: &crossterm::event::KeyEvent) -> Option<Action> {
    SHORTCUTS.iter().find_map(|s| {
        let combo = s.direct?;
        match_combo(combo, direct_mod, key).map(s.build)
    })
}
