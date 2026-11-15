//! Spike PTY: dos paneles con programas interactivos dentro de ratatui.
//!
//! Uso:
//!   pty-spike [comando args...]   arriba `comando` (por defecto nvim), abajo $SHELL
//!   pty-spike --check             comprobaciones automáticas sin terminal real
//!
//! Atajos: Alt+j / Alt+k cambian el foco, Alt+q sale. Todo lo demás va al panel.

mod check;
mod keys;
mod pane;

use std::io;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};
use ratatui::DefaultTerminal;
use tui_term::widget::{Cursor, PseudoTerminal};

use pane::Pane;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--check") {
        return check::run_all();
    }
    if args.first().map(String::as_str) == Some("--queries") {
        return check::queries(&args[1], &args[2..], 3);
    }
    let (program, prog_args) = match args.split_first() {
        Some((p, rest)) => (p.clone(), rest.to_vec()),
        None => ("nvim".to_string(), Vec::new()),
    };

    let mut terminal = ratatui::init(); // raw mode + pantalla alternativa + panic hook
    execute!(io::stdout(), EnableBracketedPaste)?;
    let result = run(&mut terminal, &program, &prog_args);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

fn areas(full: Rect) -> [Rect; 3] {
    let [top, bottom, status] = Layout::vertical([
        Constraint::Percentage(65),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(full);
    [top, bottom, status]
}

fn inner(r: Rect) -> (u16, u16) {
    (
        r.height.saturating_sub(2).max(1),
        r.width.saturating_sub(2).max(1),
    )
}

fn run(terminal: &mut DefaultTerminal, program: &str, prog_args: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let size = terminal.size()?;
    let [top, bottom, _] = areas(Rect::new(0, 0, size.width, size.height));
    let (wake_tx, wake_rx) = mpsc::channel();

    let (r, c) = inner(top);
    let mut panes = vec![Pane::spawn(
        program,
        prog_args,
        &cwd,
        r,
        c,
        wake_tx.clone(),
    )?];
    let (r, c) = inner(bottom);
    panes.push(Pane::spawn(&shell, &[], &cwd, r, c, wake_tx)?);
    let titles = [program.to_string(), shell];
    let mut focus = 0usize;
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|f| {
                let [top, bottom, status] = areas(f.area());
                for (i, area) in [top, bottom].into_iter().enumerate() {
                    let (r, c) = inner(area);
                    let _ = panes[i].resize(r, c);
                    let focused = i == focus;
                    let exited = panes[i].has_exited();
                    let title = if exited {
                        format!(" {} [terminado] ", titles[i])
                    } else {
                        format!(" {} ", titles[i])
                    };
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .border_style(if focused {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        });
                    if let Ok(parser) = panes[i].parser.lock() {
                        let hide = !focused || parser.screen().hide_cursor();
                        let widget = PseudoTerminal::new(parser.screen())
                            .block(block)
                            .cursor(Cursor::default().visibility(!hide));
                        f.render_widget(widget, area);
                    }
                }
                f.render_widget(
                    Line::from(" Alt+j/k foco · Alt+q salir · todo lo demás va al panel ")
                        .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
                    status,
                );
            })?;
            dirty = false;
        }

        // Esperar input del usuario como mucho 16 ms; mientras, salida de los PTY.
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.modifiers == KeyModifiers::ALT {
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Char('j') | KeyCode::Char('k') => {
                                focus = 1 - focus;
                                dirty = true;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    let (app_cursor, _) = panes[focus].input_modes();
                    if let Some(bytes) = keys::encode_key(key, app_cursor) {
                        panes[focus].write(&bytes)?;
                    }
                }
                Event::Paste(text) => {
                    let (_, bracketed) = panes[focus].input_modes();
                    if bracketed {
                        panes[focus].write(format!("\x1b[200~{text}\x1b[201~").as_bytes())?;
                    } else {
                        panes[focus].write(text.as_bytes())?;
                    }
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }
        while wake_rx.try_recv().is_ok() {
            dirty = true;
        }
    }
}
