# qorvi

IDE de terminal, escrito en Rust, para trabajar con varios agentes de IA en paralelo.

> **Estado: primera versión en pruebas.** La app funciona de punta a punta, pero aún no
> se ha probado con agentes reales en todas las plataformas.

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

## Uso

Dentro de un repositorio git, `qorvi` abre la app. Necesitas al menos un agente ACP en
la config (ver [`examples/config.example.toml`](examples/config.example.toml)).

- **Tareas**: `n` (con el prefijo, ver abajo) crea una; `1`…`9` cambia entre ellas. Al
  activar una tarea arrancan su agente (reanudando su sesión), su editor y su terminal.
- **Agente**: Enter envía, Esc cancela el turno, ↑/↓ + Enter responde a un permiso y
  Shift+Tab cambia el modo de permisos del agente.
- **Editor y terminal**: reciben teclado, ratón y pegado. Si el programa termina, Enter
  lo vuelve a abrir. Enter en el explorador abre el archivo en el editor.
- **Review** (`d`): diff real de la tarea; `c` commit, `m` merge en la rama base (en el
  árbol principal, que debe estar en esa rama), `p` push + PR con `gh`, `x` descartar.
- **Atajos**: `Ctrl+Space` + tecla (o `Alt`+tecla, o `Cmd`/`Ctrl` directos si la terminal
  los envía). `Ctrl+Space ?` muestra todos. `qorvi doctor keys` diagnostica la terminal.

Las tareas que trabajan en segundo plano avisan cuando terminan, fallan o piden permiso.
Merge y push + PR corren en segundo plano, con un spinner en la barra de estado; mientras
duran no se puede lanzar otra operación de la review y salir pide confirmación.

También puedes gestionar tareas desde la línea de comandos:

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

Otros comandos: `qorvi --demo` (la interfaz con datos falsos) y `qorvi doctor keys`
(diagnóstico de atajos de la terminal).

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
