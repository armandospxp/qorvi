use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use qorvi_tui::action::Action;
use qorvi_tui::demo::{demo_state, fake_changes};
use qorvi_tui::state::{
    AppState, ChatItem, ConfirmAction, Modal, Pane, ShortcutStyle, Toast, ToastLevel,
};
use qorvi_tui::update::update;
use qorvi_tui::view::view;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::text::Line;
use ratatui::Terminal;

fn state() -> AppState {
    let mut s = demo_state();
    s.ascii = false;
    // Fijo: el resultado no puede depender del sistema operativo ni de QORVI_KEYS.
    s.shortcut_style = ShortcutStyle::Direct;
    s.direct_mod = KeyModifiers::CONTROL;
    s.prefix_pending = false;
    s
}

fn render(state: &AppState, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend de prueba");
    terminal.draw(|f| view(state, f)).expect("dibujar");
    terminal.backend().buffer().clone()
}

fn text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        let mut skip = 0usize;
        for x in 0..area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf[(x, y)].symbol();
            skip = Line::from(sym).width().saturating_sub(1);
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn snap(state: &AppState, w: u16, h: u16) -> String {
    text(&render(state, w, h))
}

#[test]
fn demo_70x20() {
    insta::assert_snapshot!(snap(&state(), 70, 20));
}

#[test]
fn demo_90x28() {
    insta::assert_snapshot!(snap(&state(), 90, 28));
}

#[test]
fn demo_120x35() {
    insta::assert_snapshot!(snap(&state(), 120, 35));
}

#[test]
fn demo_140x40() {
    insta::assert_snapshot!(snap(&state(), 140, 40));
}

#[test]
fn review_120x35() {
    let mut s = state();
    s.modal = Modal::Review {
        files: fake_changes(),
        selected: 0,
    };
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn new_task_modal_90x28() {
    let mut s = state();
    s.modal = Modal::NewTask {
        name: "APP-14".to_string(),
        agent_idx: 1,
        base: "main".to_string(),
        field: 0,
    };
    insta::assert_snapshot!(snap(&s, 90, 28));
}

#[test]
fn explorer_focused_120x35() {
    let mut s = state();
    s.focus = Pane::Explorer;
    let t = &mut s.tasks[0];
    t.explorer.expanded.insert("src/util".to_string());
    t.explorer.selected = 9;
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn explorer_toggled_on_90x28() {
    let mut s = state();
    s.explorer_toggled = true;
    s.focus = Pane::Explorer;
    insta::assert_snapshot!(snap(&s, 90, 28));
}

#[test]
fn compact_explorer_80x24() {
    let mut s = state();
    s.focus = Pane::Explorer;
    insta::assert_snapshot!(snap(&s, 80, 24));
}

#[test]
fn explorer_marks_are_visible_and_deleted_is_crossed_out() {
    use ratatui::style::Modifier;
    let mut s = state();
    s.active = 1;
    let buf = render(&s, 120, 35);
    let rows = text(&buf);
    let (y, line) = rows
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("legacy.rs"))
        .expect("fila de legacy.rs");
    assert!(line.contains("D legacy.rs"), "{line}");
    let x = line.chars().position(|c| c == 'l').expect("columna");
    assert!(buf[(x as u16, y as u16)]
        .modifier
        .contains(Modifier::CROSSED_OUT));
    assert!(rows.contains("M config.rs"));
}

#[test]
fn compact_agent_80x24() {
    let mut s = state();
    s.focus = Pane::Agent;
    insta::assert_snapshot!(snap(&s, 80, 24));
}

#[test]
fn too_small_30x8() {
    insta::assert_snapshot!(snap(&state(), 30, 8));
}

#[test]
fn permission_task_with_toast_120x35() {
    let mut s = state();
    s.active = 1;
    s.toasts.push(Toast {
        text: "Tarea APP-13 esperando permiso".to_string(),
        level: ToastLevel::Warning,
        ttl_ticks: 10,
    });
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn prefix_pending_120x35() {
    let mut s = state();
    s.shortcut_style = ShortcutStyle::Prefix;
    s.prefix_pending = true;
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn help_prefix_style_120x35() {
    let mut s = state();
    s.shortcut_style = ShortcutStyle::Prefix;
    s.modal = Modal::Help;
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn help_direct_mac_120x35() {
    let mut s = state();
    s.shortcut_style = ShortcutStyle::Direct;
    s.direct_mod = KeyModifiers::SUPER;
    s.modal = Modal::Help;
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn chat_scrolled_120x35() {
    let mut s = state();
    let t = &mut s.tasks[0];
    t.chat = (1..=30)
        .map(|i| ChatItem::Agent(format!("Mensaje de prueba número {i}.")))
        .collect();
    t.chat_scroll = 7;
    insta::assert_snapshot!(snap(&s, 120, 35));
}

#[test]
fn mouse_never_panics() {
    let modals = [
        Modal::None,
        Modal::Help,
        Modal::NewTask {
            name: "x".to_string(),
            agent_idx: 0,
            base: "main".to_string(),
            field: 0,
        },
        Modal::Confirm {
            message: "¿Descartar?".to_string(),
            action: ConfirmAction::DiscardTask(0),
        },
        Modal::Review {
            files: fake_changes(),
            selected: 0,
        },
    ];
    let sizes = [
        (0, 0),
        (1, 1),
        (10, 5),
        (40, 10),
        (45, 14),
        (70, 20),
        (90, 28),
        (120, 35),
        (140, 40),
    ];
    for modal in &modals {
        for zoom in [false, true] {
            for focus in [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal] {
                for (w, h) in sizes {
                    let mut s = state();
                    s.size = (w, h);
                    s.modal = modal.clone();
                    s.zoom = zoom;
                    s.focus = focus;
                    s.tasks[0].explorer.expanded.insert("src".to_string());
                    for x in (0..w + 3).step_by(3) {
                        for y in 0..h + 3 {
                            for kind in [
                                MouseEventKind::Down(MouseButton::Left),
                                MouseEventKind::ScrollUp,
                                MouseEventKind::ScrollDown,
                            ] {
                                let m = MouseEvent {
                                    kind,
                                    column: x,
                                    row: y,
                                    modifiers: KeyModifiers::NONE,
                                };
                                update(&mut s, Action::Mouse(m));
                                // Todo clic deja un estado que se puede dibujar.
                                if x % 30 == 0 && y % 7 == 0 && w > 0 && h > 0 {
                                    render(&s, w, h);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn render_never_panics() {
    let modals = [
        Modal::None,
        Modal::Help,
        Modal::NewTask {
            name: "una-tarea-con-un-nombre-muy-largo-para-forzar-el-recorte".to_string(),
            agent_idx: 0,
            base: "main".to_string(),
            field: 2,
        },
        Modal::Confirm {
            message: "¿Descartar la tarea X? Se borrarán su worktree y su rama.".to_string(),
            action: ConfirmAction::DiscardTask(0),
        },
        Modal::Review {
            files: fake_changes(),
            selected: 1,
        },
    ];
    let mut sizes: Vec<(u16, u16)> = Vec::new();
    for w in 1..=45 {
        for h in 1..=14 {
            sizes.push((w, h));
        }
    }
    sizes.extend([
        (70, 20),
        (89, 19),
        (90, 20),
        (119, 34),
        (120, 35),
        (200, 60),
        (300, 100),
    ]);

    for modal in &modals {
        for zoom in [false, true] {
            for focus in [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal] {
                for (minimized, explorer_toggled) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    let mut s = state();
                    s.explorer_toggled = explorer_toggled;
                    s.prefix_pending = true;
                    s.shortcut_style = if zoom {
                        ShortcutStyle::Prefix
                    } else {
                        ShortcutStyle::Direct
                    };
                    s.direct_mod = if minimized {
                        KeyModifiers::SUPER
                    } else {
                        KeyModifiers::CONTROL
                    };
                    s.tasks[0].explorer.expanded.extend(
                        ["src", "src/util", ".github", ".github/workflows", "tests"]
                            .map(String::from),
                    );
                    s.modal = modal.clone();
                    s.zoom = zoom;
                    s.focus = focus;
                    s.terminal_minimized = minimized;
                    if let Some(t) = s.active_task_mut() {
                        t.input = "línea uno\nlínea dos con texto bastante largo para envolver"
                            .to_string();
                    }
                    s.push_toast("hola", ToastLevel::Info);
                    for (w, h) in &sizes {
                        render(&s, *w, *h);
                    }
                }
            }
        }
    }

    // Sin tareas y con avisos de todos los niveles.
    let mut s = state();
    s.tasks.clear();
    for level in [ToastLevel::Info, ToastLevel::Warning, ToastLevel::Error] {
        s.push_toast("aviso de prueba largo largo largo largo largo largo", level);
    }
    for (w, h) in &sizes {
        render(&s, *w, *h);
    }
}
