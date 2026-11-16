//! Geometría de la pantalla: la usan la vista (para dibujar) y `update` (para saber
//! qué hay bajo el ratón), así que nunca pueden discrepar.

use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};

use crate::state::{AppState, Pane};
use crate::theme::{status_label, status_symbol};
use crate::view::{text_width, tier, truncate, Tier};

/// Una tarea de la barra de tareas: su rectángulo, su índice y su texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTab {
    pub rect: Rect,
    pub index: usize,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Areas {
    pub tier: Tier,
    pub taskbar: Rect,
    pub body: Rect,
    pub statusbar: Rect,
    pub task_tabs: Vec<TaskTab>,
    /// No cabe ninguna tarea completa: solo se muestra la activa, cortada con `…`.
    pub task_overflow: bool,
    /// Fila de pestañas del layout compacto.
    pub tabs_bar: Option<Rect>,
    pub compact_tabs: Vec<(Rect, Pane)>,
    pub explorer: Option<Rect>,
    pub editor: Option<Rect>,
    pub agent: Option<Rect>,
    pub terminal: Option<Rect>,
    /// La terminal se dibuja como una sola fila.
    pub terminal_minimized: bool,
}

impl Areas {
    fn empty(tier: Tier) -> Self {
        Areas {
            tier,
            taskbar: Rect::default(),
            body: Rect::default(),
            statusbar: Rect::default(),
            task_tabs: Vec::new(),
            task_overflow: false,
            tabs_bar: None,
            compact_tabs: Vec::new(),
            explorer: None,
            editor: None,
            agent: None,
            terminal: None,
            terminal_minimized: false,
        }
    }

    fn set_pane(&mut self, pane: Pane, rect: Rect) {
        match pane {
            Pane::Explorer => self.explorer = Some(rect),
            Pane::Editor => self.editor = Some(rect),
            Pane::Agent => self.agent = Some(rect),
            Pane::Terminal => self.terminal = Some(rect),
        }
    }

    /// Paneles visibles en orden de dibujo.
    pub fn panes(&self) -> Vec<(Pane, Rect)> {
        [
            (Pane::Explorer, self.explorer),
            (Pane::Editor, self.editor),
            (Pane::Agent, self.agent),
            (Pane::Terminal, self.terminal),
        ]
        .into_iter()
        .filter_map(|(p, r)| r.map(|r| (p, r)))
        .collect()
    }

    pub fn pane_at(&self, col: u16, row: u16) -> Option<(Pane, Rect)> {
        let pos = Position::new(col, row);
        self.panes().into_iter().find(|(_, r)| r.contains(pos))
    }
}

/// Interior de un panel con borde.
pub fn inner(area: Rect) -> Rect {
    area.inner(Margin::new(1, 1))
}

const PANES: [Pane; 4] = [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal];

/// Textos de la fila de pestañas compactas: (inicio, separador, `tight`).
pub fn tab_chrome(width: u16) -> (&'static str, &'static str, bool) {
    // Por debajo de 48 columnas no caben las cuatro pestañas con holgura.
    if width < 48 {
        ("", "|", true)
    } else {
        (" ", " | ", false)
    }
}

pub fn tab_label(pane: Pane, tight: bool) -> String {
    if tight {
        pane.label().to_string()
    } else {
        format!(" {} ", pane.label())
    }
}

fn clamp_u16(v: usize) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

fn compact_tabs(bar: Rect) -> Vec<(Rect, Pane)> {
    let (lead, sep, tight) = tab_chrome(bar.width);
    let mut x = usize::from(bar.x) + text_width(lead);
    let mut out = Vec::new();
    for (i, pane) in PANES.into_iter().enumerate() {
        if i > 0 {
            x += text_width(sep);
        }
        let w = text_width(&tab_label(pane, tight));
        let rect =
            Rect::new(clamp_u16(x), bar.y, clamp_u16(w), bar.height.min(1)).intersection(bar);
        if rect.area() > 0 {
            out.push((rect, pane));
        }
        x += w;
    }
    out
}

fn task_label(state: &AppState, i: usize, short: bool) -> String {
    let t = &state.tasks[i];
    let sym = status_symbol(t.status, state.ascii);
    if short {
        format!("[{} {} {}]", i + 1, t.name, sym)
    } else {
        format!("[{} {} {} {}]", i + 1, t.name, sym, status_label(t.status))
    }
}

/// Devuelve las tareas visibles y si hubo que recortar a solo la activa.
fn task_tabs(state: &AppState, bar: Rect) -> (Vec<TaskTab>, bool) {
    let width = usize::from(bar.width);
    let n = state.tasks.len();
    let total = |short_inactive: bool| -> usize {
        (0..n)
            .map(|i| text_width(&task_label(state, i, short_inactive && i != state.active)))
            .sum::<usize>()
            + n.saturating_sub(1)
            + 1
    };
    let short_inactive = total(false) > width;
    // Sin tareas se dibuja " (sin tareas)": 13 columnas.
    let line_width = if n == 0 { 13 } else { total(short_inactive) };
    if line_width > width {
        if state.active >= n {
            return (Vec::new(), true);
        }
        let text = truncate(
            &format!(" {}", task_label(state, state.active, false)),
            width,
        );
        let tab = TaskTab {
            rect: Rect::new(
                bar.x,
                bar.y,
                clamp_u16(text_width(&text)),
                bar.height.min(1),
            ),
            index: state.active,
            label: text,
        };
        return (vec![tab], true);
    }
    let mut x = usize::from(bar.x) + 1;
    let mut tabs = Vec::with_capacity(n);
    for i in 0..n {
        let label = task_label(state, i, short_inactive && i != state.active);
        let w = text_width(&label);
        tabs.push(TaskTab {
            rect: Rect::new(clamp_u16(x), bar.y, clamp_u16(w), bar.height.min(1)),
            index: i,
            label,
        });
        x += w + 1;
    }
    (tabs, false)
}

pub fn compute(area: Rect, state: &AppState) -> Areas {
    let tier = tier(area.width, area.height);
    if tier == Tier::TooSmall {
        return Areas::empty(tier);
    }
    let [taskbar, body, statusbar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut a = Areas::empty(tier);
    a.taskbar = taskbar;
    a.body = body;
    a.statusbar = statusbar;
    (a.task_tabs, a.task_overflow) = task_tabs(state, taskbar);

    if state.zoom {
        a.set_pane(state.focus, body);
        return a;
    }
    if tier == Tier::Compact {
        let [tabs, pane] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
        a.tabs_bar = Some(tabs);
        a.compact_tabs = compact_tabs(tabs);
        a.set_pane(state.focus, pane);
        return a;
    }

    let minimized = (tier == Tier::Medium) ^ state.terminal_minimized;
    let term_h = if minimized {
        1
    } else {
        (body.height * 30 / 100).max(6)
    };
    let term_h = term_h.min(body.height.saturating_sub(4)).max(1);
    let [top, term] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(term_h)]).areas(body);
    // Como la terminal: el valor por defecto depende del tier y Alt+e lo invierte.
    let show_explorer = (tier == Tier::Full) ^ state.explorer_toggled;
    let rest = if show_explorer {
        // 20 % del ancho, entre 20 y 32 columnas (sin pasar de la mitad).
        let w = (top.width / 5).clamp(20, 32).min(top.width / 2);
        let [explorer, rest] =
            Layout::horizontal([Constraint::Length(w), Constraint::Min(0)]).areas(top);
        a.explorer = Some(explorer);
        rest
    } else {
        top
    };
    let [editor, agent] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(rest);
    a.editor = Some(editor);
    a.agent = Some(agent);
    a.terminal = Some(term);
    a.terminal_minimized = minimized;
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_state;
    use crate::view::view;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn state_at(w: u16, h: u16) -> AppState {
        let mut s = demo_state();
        s.ascii = false;
        s.size = (w, h);
        s
    }

    fn render(s: &AppState) -> Buffer {
        let (w, h) = s.size;
        let mut t = Terminal::new(TestBackend::new(w, h)).expect("backend de prueba");
        t.draw(|f| view(s, f)).expect("dibujar");
        t.backend().buffer().clone()
    }

    fn row_text(buf: &Buffer, rect: Rect) -> String {
        (rect.x..rect.right())
            .map(|x| buf[(x, rect.y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn task_tab_rects_cover_their_drawn_labels() {
        for (w, h) in [(120, 35), (90, 28), (70, 20), (45, 12)] {
            let s = state_at(w, h);
            let areas = compute(Rect::new(0, 0, w, h), &s);
            let buf = render(&s);
            assert!(!areas.task_tabs.is_empty(), "{w}x{h}");
            for tab in &areas.task_tabs {
                // Las celdas tras un carácter ancho (⏳) salen como espacio.
                let strip = |t: String| t.replace(' ', "");
                assert_eq!(strip(row_text(&buf, tab.rect)), strip(tab.label.clone()));
            }
        }
    }

    #[test]
    fn compact_tab_rects_cover_their_drawn_labels() {
        for w in [40, 47, 48, 80] {
            let mut s = state_at(w, 24);
            s.set_layout(true, false);
            let areas = compute(Rect::new(0, 0, w, 24), &s);
            let buf = render(&s);
            assert_eq!(areas.compact_tabs.len(), 4, "{w}");
            for (rect, pane) in &areas.compact_tabs {
                let (_, _, tight) = tab_chrome(w);
                assert_eq!(row_text(&buf, *rect), tab_label(*pane, tight));
            }
        }
    }

    #[test]
    fn panes_stay_inside_the_body_and_do_not_overlap() {
        for (w, h) in [(120, 35), (140, 40), (90, 28), (100, 30), (80, 24)] {
            for zoom in [false, true] {
                let mut s = state_at(w, h);
                s.zoom = zoom;
                let areas = compute(Rect::new(0, 0, w, h), &s);
                let panes = areas.panes();
                for (_, r) in &panes {
                    assert_eq!(r.intersection(areas.body), *r);
                }
                for (i, (_, a)) in panes.iter().enumerate() {
                    for (_, b) in &panes[i + 1..] {
                        assert_eq!(a.intersection(*b).area(), 0);
                    }
                }
            }
        }
    }

    #[test]
    fn too_small_has_no_areas() {
        let s = state_at(30, 8);
        let areas = compute(Rect::new(0, 0, 30, 8), &s);
        assert_eq!(areas.tier, Tier::TooSmall);
        assert!(areas.panes().is_empty() && areas.task_tabs.is_empty());
        assert_eq!(areas.pane_at(0, 0), None);
    }
}
