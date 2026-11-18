# qorvi

IDE de terminal, escrito en Rust, para trabajar con varios agentes de IA en paralelo.

> **Estado: en desarrollo temprano.** Todavía no hay una versión usable.

## Qué será

Cada **tarea** tiene su propio git worktree, su propio agente y un layout fijo:

```
[APP-12 ● trabajando] [APP-13 ⏳ permiso] [fix-login ✓ listo]
┌──────────────────────┬─────────────────────┐
│ Editor ($EDITOR)     │ Agente (chat ACP)   │
├──────────────────────┴─────────────────────┤
│ Terminal                                   │
└────────────────────────────────────────────┘
```

- Varias tareas a la vez: cambias entre ellas mientras los agentes siguen trabajando y
  ves de un vistazo cuál trabaja, cuál espera permiso y cuál terminó.
- Agentes vía [Agent Client Protocol](https://agentclientprotocol.com) (Claude Code,
  Codex y cualquier agente compatible con ACP).
- Tu propio editor (nvim, helix…) y tu shell, dentro de la app.
- Al terminar una tarea: revisas el diff y luego haces merge, abres una PR o la descartas.
- macOS, Linux y Windows nativo, sin tmux.

## Uso (CLI)

La app completa todavía no existe, pero ya puedes gestionar tareas desde la línea de
comandos dentro de un repositorio git:

```bash
qorvi new APP-12                 # rama APP-12 + worktree, desde la rama actual
qorvi new fix-login --base main --agent codex
cd "$(qorvi new spike --print-path)"
qorvi ls                         # tareas, agente, estado del worktree
qorvi ls --json
qorvi rm APP-12                  # pide confirmación; se niega si hay trabajo sin integrar
qorvi rm APP-12 --keep-branch    # borra el worktree pero conserva la rama
qorvi rm --orphans               # limpia worktrees que quedaron sin tarea
```

Los worktrees viven fuera del repositorio, en el directorio de datos de qorvi
(`~/.local/share/qorvi/worktrees/` en Linux, `~/Library/Application Support/qorvi/` en
macOS, `%APPDATA%\qorvi\data\` en Windows), para que herramientas como cargo o npm no
detecten el proyecto padre. La lista de tareas se guarda en `.git/qorvi/tasks.json`.
`QORVI_CONFIG_DIR` y `QORVI_DATA_DIR` cambian ambos directorios.

Otros comandos: `qorvi --demo` (mockup de la interfaz) y `qorvi doctor keys` (diagnóstico de
atajos de la terminal).

## Seguridad

qorvi limita al worktree de la tarea las lecturas y escrituras que el agente pide **a
través de ACP**, y nunca aprueba permisos de forma automática. Pero los agentes también
tienen herramientas propias que ejecutan comandos y escriben archivos con tus permisos:
eso lo controla el modo de permisos de cada agente (`mode` en la config de cada agente
fija el modo inicial). **Un worktree no es un sandbox.**

## Desarrollo

Requisitos: Rust stable y git ≥ 2.38. Para abrir PRs, [GitHub CLI](https://cli.github.com) (`gh`).

```bash
cargo build --workspace
cargo test --workspace
```

| Crate | Responsabilidad |
|---|---|
| `qorvi-core` | tareas, configuración, persistencia y git |
| `qorvi-pty` | paneles de terminal (PTY + emulador) |
| `qorvi-acp` | cliente ACP |
| `qorvi-tui` | interfaz |
| `qorvi` | binario y CLI |
| `spikes/*` | prototipos de exploración (no forman parte del producto) |

Configuración de ejemplo: [`examples/config.example.toml`](examples/config.example.toml).

## Licencia

MIT
