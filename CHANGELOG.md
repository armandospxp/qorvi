# Changelog

Todos los cambios notables de qorvi se documentan aquí.
Formato: [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/);
versionado: [SemVer](https://semver.org/lang/es/).

## [Unreleased]

### Añadido
- Esqueleto del workspace y CI multiplataforma.
- Spike de PTY (portable-pty + vt100 + tui-term) con respuesta a consultas de terminal.
- Spike de ACP: cliente con SDK 5.3 sobre `ByteStreams`, contención de rutas `fs/*` y agente fixture.
- Mockup navegable de la TUI con datos falsos (`qorvi --demo`), con explorador de archivos.
- Atajos con tecla prefijo `Ctrl+Space` + tecla (por defecto en macOS) además de `Alt+tecla` (por defecto en Windows/Linux); `Ctrl+Space q` sale siempre.
- Ratón en la TUI: clic para enfocar paneles, cambiar de tarea, usar el explorador y elegir opciones de permiso; rueda para desplazar el chat y el explorador.
- Atajos directos `Cmd` (macOS) / `Ctrl` (Windows/Linux) con detección automática del protocolo de teclado kitty; `qorvi doctor keys` diagnostica la terminal y da la config para liberar atajos en Ghostty.
- Núcleo (`qorvi-core`): ejecutor de `git` sin shell ni prompts y descubrimiento del repositorio principal (también desde el worktree de una tarea).
- Configuración global y por repositorio (`.qorvi.toml`) con fusión campo a campo; rutas de config y datos por plataforma (`QORVI_CONFIG_DIR`, `QORVI_DATA_DIR`). Los worktrees de tareas viven fuera del repo, en el directorio de datos.
- Tareas persistidas en `.git/qorvi/tasks.json` con escritura atómica y lock entre procesos.
- Operaciones git de worktrees, ramas y estado de archivos (`status --porcelain=v2`).
- Git: cambios de una tarea desde el merge-base (incluidos los no commiteados y untracked), diff por archivo y lista de archivos para el explorador.
- Git: commit, merge (merge, squash o fast-forward, abortando limpio si hay conflicto), push, borrado de ramas remotas y PRs con `gh`.
- `Workspace`: crear y borrar tareas sin perder trabajo (sin `--force` se niega con cambios sin commitear o commits sin integrar) y detectar worktrees huérfanos.
- CLI: `qorvi new`, `qorvi ls` (`--json`) y `qorvi rm` (`--keep-branch`, `--orphans`), con `-C <dir>`.
- Paneles de terminal (`qorvi-pty`): programa en un PTY con pantalla emulada, respuestas a consultas de terminal (cursor, DA1/DA2, XTVERSION, colores), código de salida, título, campana y pegado seguro con bracketed paste.
- Ratón en los paneles (X10, UTF-8 y SGR, según lo que pida el programa) y scrollback; la rueda desplaza el historial o envía flechas en pantallas como less.
- Editor de la tarea: nvim se lanza con `--listen` para abrir archivos desde el explorador; vim y helix, con teclas (best effort).
- Cliente ACP (`qorvi-acp`): cada agente corre en su propio hilo con una API síncrona y eventos propios. Sesiones nuevas o reanudadas (`session/load`, con sesión nueva y aviso si falla), turnos, cancelación, plan, herramientas con sus diffs, uso de contexto, coste y título.
- Permisos sin bloquear: se responden cuando el usuario quiera y, al cancelar el turno, los pendientes se descartan como pide ACP.
- `fs/read_text_file` (con `line`/`limit`) y `fs/write_text_file` limitados al worktree, también para rutas nuevas y symlinks; cada rechazo se notifica.
- Modo de permisos inicial por agente (`mode` en `[agents.<nombre>]`) y cambio de modo durante la sesión.
- El coste y el título de la sesión se guardan en la tarea (`qorvi ls --json`).
- `qorvi new` avisa si un archivo de `copy_files` no está en `.gitignore` (aparecería como cambio de la tarea).
- Opción `[keys] style` en la config; `QORVI_KEYS` sigue teniendo prioridad.
- **La app**: `qorvi` sin argumentos, dentro de un repositorio, abre la interfaz real. Cada tarea tiene su agente ACP (que reanuda su sesión), su editor y su terminal en PTY, y el explorador con las marcas de git.
- Chat con el agente: mensajes en streaming, herramientas, permisos, cancelación (Esc) y cambio de modo de permisos (Shift+Tab); coste y título en la barra de estado y en `tasks.json`.
- Editor y terminal reales: teclado, ratón, rueda, pegado y cursor; Enter reabre un programa que terminó; abrir archivos desde el explorador.
- Review con el diff real de la tarea: commit, merge en la rama base, push + PR con `gh` y descartar la tarea.
- Avisos de tareas en segundo plano (terminó, falló, pide permiso).
- El chat de cada tarea se guarda (`.git/qorvi/chats/<tarea>.json`, últimos 500 mensajes) y se recupera al volver a abrir qorvi; si el agente repite el historial al reanudar, no se duplica.
- El chat muestra el plan del agente (se actualiza en su sitio) y su razonamiento (atenuado, últimas 3 líneas).
- La ayuda incluye Shift+Tab (modo de permisos) y Enter para reabrir el editor o la terminal.
- Al cerrar un agente se terminan también sus procesos hijos (grupo de procesos en Unix, Job Object en Windows).
- Merge y push + PR corren en segundo plano con un spinner en la barra de estado y en la review; la interfaz sigue respondiendo, no se puede lanzar otra operación hasta que acabe y salir pide confirmación.
- Aviso, una vez por sesión, si la terminal escribe símbolos (`˙`, `∆`…) en lugar de enviar Alt.
- Pegado con bracketed paste en el chat, el diálogo de tarea nueva y los paneles.

- Config de ejemplo con los comandos de Claude Code (`claude-agent-acp`) y Codex (`codex-acp`).
- El título del panel del agente muestra el modo de permisos actual.
- Si no hay editor configurado ni `$VISUAL`/`$EDITOR`, se usa el primero que haya en el `PATH` de nvim, hx, vim, micro, nano y vi (en Windows, también `edit`).

### Cambiado
- Las dependencias se compilan optimizadas también en debug: los tests de la TUI pasan de ~90 s a ~10 s.

### Corregido
- Si el agente se cierra con error, el aviso incluye su código de salida y las últimas líneas de su stderr (antes solo decía "Incoming transport closed").
- La CI de Windows fallaba por imports que solo se usan en Unix.
