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

## Seguridad

qorvi limita al worktree de la tarea las lecturas y escrituras que el agente pide **a
través de ACP**, y nunca aprueba permisos de forma automática. Pero los agentes también
tienen herramientas propias que ejecutan comandos y escriben archivos con tus permisos:
eso lo controla el modo de permisos de cada agente. **Un worktree no es un sandbox.**

## Desarrollo

Requisitos: Rust stable y git.

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
