//! Lógica de los subcomandos `new`, `ls` y `rm`, separada de `main` para poder testearla.

use std::io::{BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use qorvi_core::{Git, NewTask, RemoveOptions, Task, Workspace};
use serde::Serialize;

/// Cómo pedir confirmación antes de borrar.
pub struct Confirm<'a> {
    yes: bool,
    interactive: bool,
    input: &'a mut dyn BufRead,
}

impl<'a> Confirm<'a> {
    pub fn new(yes: bool, interactive: bool, input: &'a mut dyn BufRead) -> Self {
        Confirm {
            yes,
            interactive,
            input,
        }
    }

    /// `Ok(true)` si se debe continuar. Falla si hace falta preguntar y no hay terminal.
    fn ask(&mut self, question: &str, out: &mut impl Write) -> anyhow::Result<bool> {
        if self.yes {
            return Ok(true);
        }
        if !self.interactive {
            bail!("hace falta confirmar el borrado: usa --yes (-y) cuando no hay terminal");
        }
        write!(out, "{question} [s/N] ")?;
        out.flush()?;
        let mut line = String::new();
        self.input.read_line(&mut line)?;
        Ok(matches!(
            line.trim().to_lowercase().as_str(),
            "s" | "si" | "sí" | "y" | "yes"
        ))
    }
}

pub fn new_task(
    ws: &Workspace,
    name: String,
    base: Option<String>,
    agent: Option<String>,
    print_path: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> anyhow::Result<()> {
    let created = ws.create_task(NewTask { name, base, agent })?;
    for w in &created.warnings {
        writeln!(err, "aviso: {w}")?;
    }
    let t = &created.task;
    if print_path {
        writeln!(out, "{}", t.worktree.display())?;
    } else {
        writeln!(
            out,
            "Tarea {} creada: rama {}, base {}, worktree {}",
            t.name.as_str(),
            t.branch,
            t.base_ref,
            t.worktree.display()
        )?;
        writeln!(out, "Entra con: cd {}", t.worktree.display())?;
    }
    Ok(())
}

/// Cambios del worktree de `task`: `None` si el directorio no existe.
fn changes(task: &Task) -> anyhow::Result<Option<usize>> {
    if !task.worktree.is_dir() {
        return Ok(None);
    }
    let n = Git::new(&task.worktree)
        .status()
        .with_context(|| format!("no se pudo leer el estado de {}", task.name.as_str()))?
        .len();
    Ok(Some(n))
}

#[derive(Serialize)]
struct TaskJson<'a> {
    #[serde(flatten)]
    task: &'a Task,
    changes: Option<usize>,
}

pub fn ls(
    ws: &Workspace,
    json: bool,
    now: SystemTime,
    out: &mut impl Write,
    err: &mut impl Write,
) -> anyhow::Result<()> {
    let tasks = ws.tasks()?;
    let rows: Vec<(Task, Option<usize>)> = tasks
        .into_iter()
        .map(|t| changes(&t).map(|c| (t, c)))
        .collect::<anyhow::Result<_>>()?;
    if json {
        let items: Vec<TaskJson> = rows
            .iter()
            .map(|(task, changes)| TaskJson {
                task,
                changes: *changes,
            })
            .collect();
        serde_json::to_writer_pretty(&mut *out, &items)?;
        writeln!(out)?;
    } else if rows.is_empty() {
        writeln!(out, "No hay tareas. Crea una con: qorvi new <nombre>")?;
    } else {
        let now = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut table = vec![vec![
            "NOMBRE".to_string(),
            "AGENTE".into(),
            "BASE".into(),
            "CREADA".into(),
            "ESTADO".into(),
            "RUTA".into(),
        ]];
        for (t, c) in &rows {
            table.push(vec![
                t.name.as_str().to_string(),
                t.agent.clone(),
                t.base_ref.clone(),
                relative_time(t.created_at, now),
                status_label(*c),
                t.worktree.display().to_string(),
            ]);
        }
        out.write_all(format_table(&table).as_bytes())?;
    }
    let orphans = ws.orphan_worktrees()?;
    if !orphans.is_empty() {
        writeln!(err, "aviso: hay worktrees huérfanos (sin tarea):")?;
        for o in &orphans {
            writeln!(err, "  {}", o.path.display())?;
        }
        writeln!(err, "Límpialos con: qorvi rm --orphans")?;
    }
    Ok(())
}

pub fn rm_task(
    ws: &Workspace,
    name: &str,
    force: bool,
    keep_branch: bool,
    mut confirm: Confirm,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let what = if keep_branch {
        "worktree"
    } else {
        "worktree y rama"
    };
    if !confirm.ask(&format!("¿Borrar la tarea {name} ({what})?"), out)? {
        writeln!(out, "Cancelado.")?;
        return Ok(());
    }
    let task = ws.remove_task(name, RemoveOptions { force, keep_branch })?;
    writeln!(out, "Tarea {} borrada.", task.name.as_str())?;
    Ok(())
}

pub fn rm_orphans(
    ws: &Workspace,
    mut confirm: Confirm,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let orphans = ws.orphan_worktrees()?;
    if orphans.is_empty() {
        writeln!(out, "No hay worktrees huérfanos.")?;
        return Ok(());
    }
    writeln!(out, "Worktrees huérfanos:")?;
    for o in &orphans {
        writeln!(out, "  {}", o.path.display())?;
    }
    if !confirm.ask(
        &format!("¿Borrar {} worktree(s) huérfano(s)?", orphans.len()),
        out,
    )? {
        writeln!(out, "Cancelado.")?;
        return Ok(());
    }
    for o in &orphans {
        ws.remove_orphan(o)?;
        writeln!(out, "Borrado {}", o.path.display())?;
    }
    Ok(())
}

fn status_label(changes: Option<usize>) -> String {
    match changes {
        None => "sin worktree".to_string(),
        Some(0) => "limpio".to_string(),
        Some(1) => "1 cambio".to_string(),
        Some(n) => format!("{n} cambios"),
    }
}

/// Fecha relativa simple a partir de segundos unix.
pub fn relative_time(created: u64, now: u64) -> String {
    let secs = now.saturating_sub(created);
    if secs < 60 {
        "hace un momento".to_string()
    } else if secs < 3600 {
        format!("hace {} min", secs / 60)
    } else if secs < 86_400 {
        format!("hace {} h", secs / 3600)
    } else {
        format!("hace {} d", secs / 86_400)
    }
}

/// Tabla con columnas alineadas a la izquierda; la última no se rellena.
pub fn format_table(rows: &[Vec<String>]) -> String {
    let cols = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..cols)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let mut s = String::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i + 1 == cols {
                s.push_str(cell);
            } else {
                s.push_str(cell);
                s.push_str(&" ".repeat(widths[i] - cell.chars().count() + 2));
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fecha_relativa() {
        assert_eq!(relative_time(100, 110), "hace un momento");
        assert_eq!(relative_time(0, 300), "hace 5 min");
        assert_eq!(relative_time(0, 3 * 3600 + 10), "hace 3 h");
        assert_eq!(relative_time(0, 2 * 86_400), "hace 2 d");
        assert_eq!(relative_time(500, 100), "hace un momento");
    }

    #[test]
    fn tabla_alineada() {
        let rows = vec![
            vec!["NOMBRE".to_string(), "RUTA".into()],
            vec!["a".into(), "/x".into()],
            vec!["largo".into(), "/y".into()],
        ];
        assert_eq!(
            format_table(&rows),
            "NOMBRE  RUTA\na       /x\nlargo   /y\n"
        );
    }

    #[test]
    fn etiquetas_de_estado() {
        assert_eq!(status_label(None), "sin worktree");
        assert_eq!(status_label(Some(0)), "limpio");
        assert_eq!(status_label(Some(1)), "1 cambio");
        assert_eq!(status_label(Some(3)), "3 cambios");
    }

    #[test]
    fn confirmacion() {
        let mut out = Vec::new();
        let mut input: &[u8] = b"s\n";
        assert!(Confirm::new(false, true, &mut input)
            .ask("¿?", &mut out)
            .unwrap());
        let mut input: &[u8] = b"\n";
        assert!(!Confirm::new(false, true, &mut input)
            .ask("¿?", &mut out)
            .unwrap());
        let mut input: &[u8] = b"";
        assert!(Confirm::new(false, false, &mut input)
            .ask("¿?", &mut out)
            .is_err());
        assert!(Confirm::new(true, false, &mut input)
            .ask("¿?", &mut out)
            .unwrap());
    }
}
