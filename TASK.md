# Avance de qorvi

Estado del proyecto por fases. Marca: `[x]` hecho · `[~]` en curso · `[ ]` pendiente.
Ejecutor: **Opus** (orquestador), **Sonnet** / **Haiku** (workers), **Usuario**.
Los cambios entregados se detallan en [CHANGELOG.md](CHANGELOG.md).

_Última actualización: 2026-11-16_

## Fase 0: esqueleto y spikes ✅

- [x] Workspace de Cargo con los crates `qorvi-core`, `qorvi-pty`, `qorvi-acp`, `qorvi-tui` y `qorvi` · Opus
- [x] CI en macOS, Linux y Windows (fmt, clippy, tests) · Haiku
- [x] Config de ejemplo y CHANGELOG · Haiku
- [x] Spike PTY: nvim, less, htop y Claude Code dentro de un panel; respondedor de consultas de terminal · Opus
- [x] Spike ACP: cliente con el SDK 5.3, contención de rutas, agente fixture · Opus
- [x] Pruebas con Claude Code real: edición, cancelación, reanudación de sesión · Usuario
- [ ] Prueba con Codex real (opcional) · Usuario

## Mockup de la interfaz (`qorvi --demo`) ✅

- [x] TUI navegable con datos falsos: tareas, chat, permisos, review, diálogos, avisos · Sonnet
- [x] Layout según el tamaño de la terminal y snapshots a 70×20, 90×28, 120×35 y 140×40 · Sonnet
- [x] Explorador de archivos con marcas de cambios · Sonnet
- [x] Atajos con tecla prefijo `Ctrl+Space` (por defecto en macOS) además de `Alt` · Sonnet
- [x] Ratón: clic para enfocar paneles, tareas, explorador y permisos; rueda para desplazar · Sonnet
- [x] Atajos directos `Cmd` (macOS) / `Ctrl` (Windows/Linux) con detección automática de la terminal · Sonnet
- [ ] Opción `[keys] style` en la config (llega con la config de la fase 1; hoy: variable `QORVI_KEYS`) · Sonnet
- [ ] Probar atajos en Ghostty, iTerm2, Terminal.app, VS Code, Windows Terminal y GNOME Terminal · Usuario

## Fase 1: núcleo (`qorvi-core`) y CLI ✅

- [x] Errores y ejecutor de comandos git · Opus
- [x] Descubrir el repositorio principal · Opus
- [x] Rutas y configuración · Sonnet
- [x] Tareas y persistencia atómica · Sonnet
- [x] Git: worktrees, ramas y estado · Sonnet
- [x] Git: cambios, diff por archivo y lista de archivos · Sonnet
- [x] Git: commit, merge, push, PR y borrado · Sonnet
- [x] `Workspace`: crear y borrar tareas, worktrees huérfanos · Opus
- [x] CLI `qorvi new`, `qorvi ls`, `qorvi rm` · Sonnet
- [x] README y CHANGELOG · Opus

## Fase 2: paneles de terminal (`qorvi-pty`)

- [ ] Panel PTY con emulador y respondedor de consultas
- [ ] Reenvío de ratón y scrollback
- [ ] nvim con `--listen` para abrir archivos desde el explorador
- [ ] Prueba en Windows (ConPTY)

## Fase 3: agentes (`qorvi-acp`)

- [ ] Cliente ACP: sesiones, turnos, cancelación y reanudación
- [ ] Permisos sin bloquear y contención de `fs/*`
- [ ] Modo de permisos del agente visible y configurable
- [ ] Coste y título de sesión por tarea

## Fase 4: aplicación completa (`qorvi-tui`)

- [ ] Conectar el mockup a core, PTY y ACP
- [ ] Notificaciones de tareas en segundo plano
- [ ] Aviso si la terminal no envía Alt

## Mejoras detectadas

- [ ] El test `snapshots` de qorvi-tui tarda ~90 s: acelerarlo · Sonnet
- [ ] Avisar en `qorvi new` si un archivo de `copy_files` no está en `.gitignore` (cuenta como cambio y obliga a `rm --force`) · Sonnet
- [ ] Probar la CLI y los tests de git en macOS y Windows (canonicalización de rutas, `NUL`) · Usuario
- [ ] Probar `gh` real: `create_pr` y `pr_for_branch` · Usuario

## Decisiones pendientes

- Ninguna abierta ahora mismo.
