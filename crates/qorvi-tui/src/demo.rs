//! Datos falsos para `qorvi --demo`. Nada de aquí toca procesos, git ni red.

use crossterm::event::KeyModifiers;

use crate::explorer::{visible_rows, RowKind};
use crate::state::{
    AgentStatus, AppState, ChangeKind, ChangeMark, ChangedFile, ChatItem, FileEntry, Modal, Pane,
    ShortcutStyle, Task, ToolStatus,
};

fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn fake_editor_lines() -> Vec<String> {
    let mut v = lines(&[
        "  1 use std::io;",
        "  2 ",
        "  3 mod config;",
        "  4 mod session;",
        "  5 ",
        "  6 /// Valida el nombre de una tarea nueva.",
        "  7 pub fn validate_name(name: &str) -> Result<(), String> {",
        "  8     if name.is_empty() {",
        "  9         return Err(\"nombre vacío\".to_string());",
        " 10     }",
        " 11     if name.contains('/') || name.contains(' ') {",
        " 12         return Err(\"carácter no permitido\".to_string());",
        " 13     }",
        " 14     Ok(())",
        " 15 }",
        " 16 ",
        " 17 fn main() -> io::Result<()> {",
        " 18     let cfg = config::load()?;",
        " 19     session::run(cfg)",
        " 20 }",
    ]);
    v.extend(std::iter::repeat_n("~".to_string(), 30));
    v
}

fn fake_terminal_lines() -> Vec<String> {
    lines(&[
        "$ cargo test",
        "   Compiling qorvi-core v0.1.0",
        "    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.31s",
        "     Running unittests src/lib.rs",
        "",
        "running 4 tests",
        "test config::tests::loads_defaults ... ok",
        "test config::tests::rejects_unknown_agent ... ok",
        "test session::tests::validates_names ... ok",
        "test session::tests::resumes_task ... ok",
        "",
        "test result: ok. 4 passed; 0 failed; 0 ignored",
        "$ ",
    ])
}

/// Cambios de ejemplo para la vista de review (3 archivos).
pub fn fake_changes() -> Vec<ChangedFile> {
    vec![
        ChangedFile {
            path: "src/lib.rs".to_string(),
            kind: ChangeKind::Modified,
            diff: lines(&[
                "--- a/src/lib.rs",
                "+++ b/src/lib.rs",
                "@@ -4,7 +4,12 @@ mod session;",
                " ",
                "-pub fn validate_name(name: &str) -> bool {",
                "-    !name.is_empty()",
                "+pub fn validate_name(name: &str) -> Result<(), String> {",
                "+    if name.is_empty() {",
                "+        return Err(\"nombre vacío\".to_string());",
                "+    }",
                "+    Ok(())",
                " }",
            ]),
        },
        ChangedFile {
            path: "src/session.rs".to_string(),
            kind: ChangeKind::Added,
            diff: lines(&[
                "--- /dev/null",
                "+++ b/src/session.rs",
                "@@ -0,0 +1,6 @@",
                "+pub fn run(cfg: Config) -> io::Result<()> {",
                "+    for task in cfg.tasks {",
                "+        println!(\"{}\", task.name);",
                "+    }",
                "+    Ok(())",
                "+}",
            ]),
        },
        ChangedFile {
            path: "notas.txt".to_string(),
            kind: ChangeKind::Untracked,
            diff: lines(&[
                "--- /dev/null",
                "+++ b/notas.txt",
                "@@ -0,0 +1,2 @@",
                "+pendiente: tests de integración",
                "+pendiente: documentar el flujo",
            ]),
        },
    ]
}

/// Árbol de ejemplo: `(ruta, marca)` con `M`/`A`/`D`/`?` o `' '` sin cambios.
fn tree(items: &[(&str, char)]) -> Vec<FileEntry> {
    items
        .iter()
        .map(|(path, m)| FileEntry {
            path: path.to_string(),
            mark: match m {
                'M' => Some(ChangeMark::Modified),
                'A' => Some(ChangeMark::Added),
                'D' => Some(ChangeMark::Deleted),
                '?' => Some(ChangeMark::Untracked),
                _ => None,
            },
        })
        .collect()
}

fn task(
    name: &str,
    agent: &str,
    status: AgentStatus,
    cost: f64,
    chat: Vec<ChatItem>,
    files: &[(&str, char)],
) -> Task {
    let mut t = Task::new(name, agent, "main");
    t.files = tree(files);
    t.editor_file = Some("src/main.rs".to_string());
    t.explorer.expanded.insert("src".to_string());
    // La selección arranca en el primer archivo con cambios.
    let rows = visible_rows(&t.files, &t.explorer.expanded);
    t.explorer.selected = rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::File { mark: Some(_) }))
        .unwrap_or(0);
    t.status = status;
    t.cost_usd = cost;
    t.chat = chat;
    t.editor_lines = fake_editor_lines();
    t.terminal_lines = fake_terminal_lines();
    t
}

pub fn demo_state() -> AppState {
    let app12 = task(
        "APP-12",
        "claude",
        AgentStatus::Working,
        0.42,
        vec![
            ChatItem::User("Mejora la validación del nombre de tarea en src/lib.rs".to_string()),
            ChatItem::Agent(
                "Voy a leer el módulo actual y después aplico el cambio con errores descriptivos."
                    .to_string(),
            ),
            ChatItem::ToolCall {
                id: String::new(),
                title: "Leer src/main.rs".to_string(),
                kind: "read".to_string(),
                status: ToolStatus::Completed,
                expanded: false,
            },
            ChatItem::ToolCall {
                id: String::new(),
                title: "Editar src/lib.rs".to_string(),
                kind: "edit".to_string(),
                status: ToolStatus::InProgress,
                expanded: true,
            },
        ],
        &[
            (".github/workflows/ci.yml", ' '),
            (".gitignore", ' '),
            ("Cargo.toml", ' '),
            ("README.md", ' '),
            ("notas.txt", '?'),
            ("src/config.rs", ' '),
            ("src/lib.rs", 'M'),
            ("src/main.rs", ' '),
            ("src/session.rs", 'A'),
            ("src/util/fs.rs", ' '),
            ("src/util/mod.rs", ' '),
            ("tests/integration.rs", ' '),
        ],
    );
    let app13 = task(
        "APP-13",
        "codex",
        AgentStatus::NeedsPermission,
        0.18,
        vec![
            ChatItem::User("Corrige el test que falla en config".to_string()),
            ChatItem::Agent("Necesito ejecutar los tests para reproducir el fallo.".to_string()),
            ChatItem::Permission {
                title: "Ejecutar: cargo test".to_string(),
                options: lines(&["Permitir una vez", "Permitir siempre", "Rechazar"]),
                selected: 0,
                resolved: None,
                request: None,
            },
        ],
        &[
            (".github/workflows/ci.yml", ' '),
            (".gitignore", ' '),
            ("Cargo.lock", ' '),
            ("Cargo.toml", ' '),
            ("README.md", ' '),
            ("src/config.rs", 'M'),
            ("src/legacy.rs", 'D'),
            ("src/lib.rs", ' '),
            ("src/main.rs", ' '),
            ("src/session/mod.rs", ' '),
            ("src/session/store.rs", ' '),
            ("tests/config.rs", 'M'),
            ("tests/integration.rs", ' '),
        ],
    );
    let fix_login = task(
        "fix-login",
        "claude",
        AgentStatus::Done,
        1.07,
        vec![
            ChatItem::User("El login acepta contraseñas vacías, arréglalo".to_string()),
            ChatItem::ToolCall {
                id: String::new(),
                title: "Editar src/auth.rs".to_string(),
                kind: "edit".to_string(),
                status: ToolStatus::Completed,
                expanded: false,
            },
            ChatItem::Agent("Listo: el login ahora rechaza contraseñas vacías.".to_string()),
        ],
        &[
            (".github/workflows/ci.yml", ' '),
            (".gitignore", ' '),
            ("Cargo.toml", ' '),
            ("README.md", ' '),
            ("docs/login.md", '?'),
            ("src/auth.rs", 'M'),
            ("src/config.rs", ' '),
            ("src/lib.rs", ' '),
            ("src/login.rs", 'M'),
            ("src/main.rs", ' '),
            ("src/session/mod.rs", ' '),
            ("src/session/store.rs", ' '),
            ("tests/integration.rs", ' '),
            ("tests/login.rs", 'A'),
        ],
    );
    AppState {
        tasks: vec![app12, app13, fix_login],
        active: 0,
        focus: Pane::Agent,
        zoom: false,
        terminal_minimized: false,
        explorer_toggled: false,
        modal: Modal::None,
        toasts: Vec::new(),
        last_toast: None,
        agents: lines(&["claude", "codex"]),
        default_base: "main".to_string(),
        editor_name: "nvim".to_string(),
        shell_name: "zsh".to_string(),
        demo: true,
        should_quit: false,
        ascii: std::env::var("QORVI_ASCII").is_ok_and(|v| v == "1"),
        // El bucle de eventos fija el estilo tras detectar la terminal.
        shortcut_style: ShortcutStyle::Prefix,
        direct_mod: if cfg!(target_os = "macos") {
            KeyModifiers::SUPER
        } else {
            KeyModifiers::CONTROL
        },
        prefix_pending: false,
        alt_warned: false,
        compact: false,
        explorer_default: true,
        size: (0, 0),
    }
}
