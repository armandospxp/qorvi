# Avance de qorvi

Estado del proyecto por fases. Marca: `[x]` hecho · `[~]` en curso · `[ ]` pendiente.
Ejecutor: **Opus** (orquestador), **Sonnet** / **Haiku** (workers), **Usuario**.
Los cambios entregados se detallan en [CHANGELOG.md](CHANGELOG.md).

_Última actualización: 2026-11-18_

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
- [x] Opción `[keys] style` en la config (`QORVI_KEYS` tiene prioridad) · Opus
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

## Fase 2: paneles de terminal (`qorvi-pty`) 🚧

- [x] Panel PTY con emulador y respondedor de consultas · Opus
- [x] Reenvío de ratón y scrollback · Sonnet
- [x] nvim con `--listen` para abrir archivos desde el explorador · Sonnet
- [x] Compilación sin warnings para Windows y macOS (clippy con `--target`) · Opus
- [ ] Prueba en Windows (ConPTY) · Usuario

## Fase 3: agentes (`qorvi-acp`) ✅

- [x] Cliente ACP: sesiones, turnos, cancelación y reanudación · Opus
- [x] Permisos sin bloquear y contención de `fs/*` · Opus
- [x] Modo de permisos del agente visible y configurable (`mode` por agente) · Opus
- [x] Coste y título de sesión por tarea (`Workspace::record_session`) · Opus

## Fase 4: aplicación completa (`qorvi-tui`) 🚧

- [x] Conectar el mockup a core, PTY y ACP (`qorvi` sin argumentos abre la app) · Opus
- [x] Notificaciones de tareas en segundo plano (avisos al terminar, fallar o pedir permiso) · Opus
- [x] Aviso si la terminal no envía Alt · Opus
- [x] Prueba de punta a punta en tmux con el agente fixture: crear, permiso, fs, explorador, terminal, review, commit, merge, descartar · Opus
- [x] Probar la app con Claude Code real (reanudar, cancelar, modos, permisos, coste, review y merge) · Opus
- [ ] Probar la app con Claude Code real desde tu terminal y en un proyecto de verdad · Usuario
- [ ] Probar la app en macOS y Windows · Usuario

## Mejoras detectadas

- [x] El test `snapshots` de qorvi-tui tardaba ~90 s: ahora ~10 s (dependencias optimizadas en dev) · Opus
- [x] Avisar en `qorvi new` si un archivo de `copy_files` no está en `.gitignore` · Opus
- [x] Al cerrar un agente, matar también sus procesos hijos (grupo de procesos en Unix, Job Object en Windows) · Opus
- [x] Añadir a la config de ejemplo los comandos reales de Claude Code y Codex por ACP · Opus
- [x] Mostrar el stderr del agente cuando se cierra con error (p. ej. Node demasiado viejo) · Opus
- [x] Mostrar el modo de permisos actual en el título del panel del agente · Opus
- [x] Editor por defecto si no hay ninguno configurado (busca nvim, hx, vim… en el PATH) · Opus
- [x] Shift+Tab pide confirmación antes de pasar a un modo que aprueba todo ("Bypass permissions", acceso total) · Opus
- [x] Merge, push y PR en segundo plano con spinner · Opus
- [x] Mostrar el plan del agente (`Plan`) y su razonamiento en el chat · Opus
- [x] Persistir el chat por tarea (`.git/qorvi/chats/`) · Opus
- [x] Ayuda (`?`): añadir Shift+Tab (modo) y Enter (reabrir panel) · Opus
- [ ] Probar en Windows que el Job Object cierra los procesos hijos del agente · Usuario
- [ ] Probar la CLI y los tests de git en macOS y Windows (canonicalización de rutas, `NUL`) · Usuario
- [ ] Probar `gh` real: `create_pr` y `pr_for_branch` · Usuario

## Decisiones pendientes

- Ninguna abierta ahora mismo.
