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
