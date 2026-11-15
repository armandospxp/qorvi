use qorvi_tui::demo::{demo_state, fake_changes};
use qorvi_tui::state::{AppState, ConfirmAction, Modal, Pane, Toast, ToastLevel};
use qorvi_tui::view::view;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::text::Line;
use ratatui::Terminal;

fn state() -> AppState {
    let mut s = demo_state();
    s.ascii = false;
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
            for focus in [Pane::Editor, Pane::Agent, Pane::Terminal] {
                for minimized in [false, true] {
                    let mut s = state();
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
