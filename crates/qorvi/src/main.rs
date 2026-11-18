mod commands;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use qorvi_core::{Config, Paths, Repo, RepoPaths, Workspace};

#[derive(Parser)]
#[command(
    name = "qorvi",
    version,
    about = "IDE de terminal para agentes de IA en paralelo"
)]
struct Cli {
    /// Abre la interfaz de demostración (mockup).
    #[arg(long)]
    demo: bool,
    /// Usa este directorio en lugar del actual (como `git -C`).
    #[arg(short = 'C', global = true, value_name = "DIR")]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Crea una tarea: worktree, rama y agente.
    New {
        /// Nombre de la tarea (también de su rama y su directorio).
        name: String,
        /// Rama o ref de partida (por defecto, la rama actual).
        #[arg(long)]
        base: Option<String>,
        /// Agente a usar (por defecto, el de la configuración).
        #[arg(long)]
        agent: Option<String>,
        /// Imprime solo la ruta del worktree.
        #[arg(long)]
        print_path: bool,
    },
    /// Lista las tareas.
    Ls {
        /// Salida en JSON.
        #[arg(long)]
        json: bool,
    },
    /// Borra una tarea (worktree y rama) o los worktrees huérfanos.
    Rm {
        /// Nombre de la tarea.
        #[arg(required_unless_present = "orphans", conflicts_with = "orphans")]
        name: Option<String>,
        /// Borra aunque haya cambios sin guardar o commits sin integrar.
        #[arg(long)]
        force: bool,
        /// Conserva la rama git.
        #[arg(long)]
        keep_branch: bool,
        /// Borra los worktrees huérfanos en lugar de una tarea.
        #[arg(long)]
        orphans: bool,
        /// No pide confirmación.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Herramientas de diagnóstico.
    Doctor {
        #[command(subcommand)]
        what: Doctor,
    },
}

#[derive(Subcommand)]
enum Doctor {
    /// Diagnostica los atajos de teclado de la terminal.
    Keys,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `qorvi ls | head`: que se cierre la tubería no es un error.
            if e.chain().any(|c| {
                let kind = c
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind)
                    .or_else(|| {
                        c.downcast_ref::<serde_json::Error>()
                            .and_then(serde_json::Error::io_error_kind)
                    });
                kind == Some(std::io::ErrorKind::BrokenPipe)
            }) {
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {}", error_message(&e));
            ExitCode::from(1)
        }
    }
}

/// Mensaje con la cadena de causas, sin repetir las que el mensaje anterior ya incluye
/// (los errores de `qorvi-core` suelen llevar su causa en el propio texto).
fn error_message(e: &anyhow::Error) -> String {
    let mut msg = e.to_string();
    for cause in e.chain().skip(1) {
        let cause = cause.to_string();
        if !msg.contains(&cause) {
            msg.push_str(": ");
            msg.push_str(&cause);
        }
    }
    msg
}

fn open(dir: Option<PathBuf>) -> anyhow::Result<Workspace> {
    let start = match dir {
        Some(d) => d,
        None => std::env::current_dir().context("no se pudo leer el directorio actual")?,
    };
    Ok(Workspace::open(start, &Paths::from_env()?)?)
}

fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.demo {
        let style = key_style();
        return qorvi_tui::run_demo(style.as_deref())
            .context("fallo en la interfaz de demostración");
    }
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut err = std::io::stderr();
    match cli.command {
        None => {
            println!("qorvi {}", env!("CARGO_PKG_VERSION"));
            println!("Usa `qorvi --demo` para ver el mockup de la interfaz.");
            println!("Usa `qorvi new|ls|rm` para gestionar tareas (`qorvi --help`).");
            println!("Usa `qorvi doctor keys` para diagnosticar los atajos de tu terminal.");
            Ok(())
        }
        Some(Command::Doctor { what: Doctor::Keys }) => doctor_keys(),
        Some(Command::New {
            name,
            base,
            agent,
            print_path,
        }) => {
            let ws = open(cli.dir)?;
            commands::new_task(&ws, name, base, agent, print_path, &mut out, &mut err)
        }
        Some(Command::Ls { json }) => {
            let ws = open(cli.dir)?;
            commands::ls(&ws, json, std::time::SystemTime::now(), &mut out, &mut err)
        }
        Some(Command::Rm {
            name,
            force,
            keep_branch,
            orphans,
            yes,
        }) => {
            let ws = open(cli.dir)?;
            let mut input = std::io::stdin().lock();
            let confirm = commands::Confirm::new(yes, std::io::stdin().is_terminal(), &mut input);
            if orphans {
                commands::rm_orphans(&ws, confirm, &mut out)
            } else {
                let name = name.expect("clap exige el nombre sin --orphans");
                commands::rm_task(&ws, &name, force, keep_branch, confirm, &mut out)
            }
        }
    }?;
    out.flush().context("no se pudo escribir la salida")
}

/// Estilo de atajos pedido: `QORVI_KEYS` (para una sesión) o `[keys] style` de la config
/// global y del repositorio actual, si lo hay. Una config inválida solo avisa: la demo
/// no la necesita para nada más.
fn key_style() -> Option<String> {
    if let Some(v) = std::env::var("QORVI_KEYS")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return Some(v);
    }
    let config = Paths::from_env()
        .map_err(anyhow::Error::from)
        .and_then(|paths| {
            let repo = std::env::current_dir()
                .ok()
                .and_then(|d| Repo::discover(d).ok())
                .map(|r| RepoPaths::new(&paths, &r));
            Ok(Config::load(&paths, repo.as_ref())?)
        });
    match config {
        Ok(c) => c.keys.style.map(|s| s.as_str().to_string()),
        Err(e) => {
            eprintln!("aviso: se ignora la config: {}", error_message(&e));
            None
        }
    }
}

/// Imprime el informe de atajos; consulta a la terminal con el modo raw activo un instante.
fn doctor_keys() -> anyhow::Result<()> {
    let enhancement = qorvi_tui::probe_keyboard_enhancement_raw()
        .context("no se pudo consultar el teclado de la terminal")?;
    let term_program = std::env::var("TERM_PROGRAM").ok();
    print!(
        "{}",
        qorvi_tui::detect::doctor_keys_report(
            term_program.as_deref(),
            cfg!(target_os = "macos"),
            cfg!(windows),
            enhancement,
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mensaje_de_error_sin_causas_repetidas() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no existe");
        let core = qorvi_core::Error::io("/x", io);
        let e = anyhow::Error::new(core).context("no se pudo abrir");
        assert_eq!(error_message(&e), "no se pudo abrir: /x: no existe");
    }
}
