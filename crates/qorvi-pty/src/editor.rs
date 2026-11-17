//! Lanzar el editor del usuario en un panel de modo que el explorador pueda abrir
//! archivos en él.
//!
//! - **Neovim**: se arranca con `--listen <dirección>` y los archivos se abren desde fuera
//!   con [`NvimRemote::open_file`] (`nvim --server … --remote-expr …`). Es fiable.
//! - **Vim, Helix y otros**: no hay canal externo; [`open_file_keys`] devuelve las teclas
//!   que la TUI escribiría en el panel. Es "best effort": solo funciona si el editor está
//!   en modo normal y no hay un prompt abierto.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::pane::PaneCommand;

/// Errores del módulo del editor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// La línea de configuración del editor está vacía.
    #[error("el editor configurado está vacío")]
    EmptyEditor,
    /// Comillas sin cerrar en la línea del editor.
    #[error("comillas sin cerrar en el editor configurado: {0}")]
    UnclosedQuote(String),
    /// No se pudo preparar el socket o el directorio del servidor de nvim.
    #[error("no se pudo preparar el servidor de nvim: {0}")]
    Socket(#[from] std::io::Error),
    /// nvim no respondió o devolvió un error.
    #[error("nvim no respondió en `{addr}`: {message}")]
    Remote { addr: String, message: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Familia de editor, deducida del nombre del ejecutable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKind {
    Neovim,
    Vim,
    Helix,
    Other,
}

impl EditorKind {
    /// Detecta el editor por el nombre del ejecutable (sin ruta ni `.exe`, sin distinguir
    /// mayúsculas): `nvim`, `vim`/`vi`, `hx`/`helix`.
    pub fn detect(program: &OsStr) -> EditorKind {
        let raw = program.to_string_lossy();
        // Se parte por ambos separadores: en Unix `C:\x\nvim.exe` también debe detectarse.
        let name = raw
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&raw)
            .to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        match name {
            "nvim" => EditorKind::Neovim,
            "vim" | "vi" => EditorKind::Vim,
            "hx" | "helix" => EditorKind::Helix,
            _ => EditorKind::Other,
        }
    }
}

/// Comando del editor listo para lanzar en un panel.
#[derive(Debug, Clone)]
pub struct EditorSpec {
    pub command: PaneCommand,
    pub kind: EditorKind,
    /// Control remoto del nvim lanzado (solo para Neovim).
    pub remote: Option<NvimRemote>,
}

/// Divide una línea de comando con reglas tipo shell simples: los espacios separan, las
/// comillas simples y dobles agrupan (sin escapes ni expansión). Error si no cierran.
pub fn split_command_line(line: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                has_token = true;
            }
            None if c.is_whitespace() => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            None => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(Error::UnclosedQuote(line.to_string()));
    }
    if has_token {
        out.push(cur);
    }
    Ok(out)
}

/// Prepara el comando del editor. `editor` es la línea de configuración (`"nvim"`,
/// `"nvim -u ~/x.lua"`); `instance_id` distingue los servidores de distintas tareas.
/// Para Neovim añade `--listen <dirección>` y devuelve el control remoto.
pub fn editor_command(editor: &str, cwd: &Path, instance_id: &str) -> Result<EditorSpec> {
    let mut parts = split_command_line(editor)?.into_iter();
    let program = parts.next().ok_or(Error::EmptyEditor)?;
    let kind = EditorKind::detect(OsStr::new(&program));
    let mut command = PaneCommand::new(&program, cwd).args(parts);
    let mut remote = None;
    if kind == EditorKind::Neovim {
        let addr = nvim_address(instance_id, std::process::id())?;
        command.args.push("--listen".into());
        command.args.push(addr.clone());
        remote = Some(NvimRemote {
            program: program.into(),
            addr,
        });
    }
    Ok(EditorSpec {
        command,
        kind,
        remote,
    })
}

/// Longitud máxima que aceptamos para la ruta de un socket Unix (el límite real es ~104
/// bytes en macOS y 108 en Linux).
#[cfg(any(unix, test))]
const MAX_SOCKET_PATH: usize = 100;

/// Deja solo `[A-Za-z0-9_-]`; el resto se sustituye por `_`.
fn sanitize(instance: &str) -> String {
    let s: String = instance
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "ed".to_string()
    } else {
        s
    }
}

/// FNV-1a de 64 bits (igual que `qorvi-core/src/paths.rs`), 8 dígitos hex.
#[cfg(any(unix, test))]
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..8].to_string()
}

/// Ruta del socket dentro de `dir`. Si no cabe usa un nombre corto con hash. Función pura,
/// para poder probarla.
#[cfg(any(unix, test))]
fn socket_path_in(dir: &Path, instance: &str, pid: u32) -> PathBuf {
    let clean = sanitize(instance);
    let long = dir.join(format!("{clean}-{pid}.sock"));
    if long.as_os_str().len() <= MAX_SOCKET_PATH {
        return long;
    }
    dir.join(format!("{}-{pid}.sock", short_hash(instance)))
}

/// Dirección del servidor nvim en Windows: un named pipe.
#[cfg(windows)]
fn nvim_address(instance: &str, pid: u32) -> Result<OsString> {
    Ok(format!(r"\\.\pipe\qorvi-{pid}-{}", sanitize(instance)).into())
}

/// Dirección del servidor nvim en Unix: un socket en un directorio 0700 propio.
#[cfg(unix)]
fn nvim_address(instance: &str, pid: u32) -> Result<OsString> {
    let make_dir = |base: &Path| -> std::io::Result<PathBuf> {
        let dir = base.join(format!("qorvi-{pid}"));
        create_private_dir(&dir)?;
        Ok(dir)
    };
    let mut dir = make_dir(&std::env::temp_dir())?;
    let mut path = socket_path_in(&dir, instance, pid);
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        // Ni con hash cabe (TMPDIR muy largo): /tmp.
        dir = make_dir(Path::new("/tmp"))?;
        path = socket_path_in(&dir, instance, pid);
    }
    // Residuo de una ejecución anterior con el mismo pid.
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(path.into_os_string())
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        }
        Err(e) => Err(e),
    }
}

/// Control remoto de un nvim lanzado con `--listen`.
#[derive(Debug, Clone)]
pub struct NvimRemote {
    program: OsString,
    addr: OsString,
}

const REMOTE_TIMEOUT: Duration = Duration::from_secs(3);

impl NvimRemote {
    /// Dirección del servidor (para esperar a que exista el socket, mostrarla, etc.).
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }

    /// Abre `path` en el nvim del panel con `:drop` (salta a la ventana si ya está abierto
    /// y no pierde cambios del buffer actual), opcionalmente en la línea `line`.
    pub fn open_file(&self, path: &Path, line: Option<u32>) -> Result<()> {
        // Sale de modo inserción/terminal antes de ejecutar nada.
        self.run("--remote-send", r"<C-\><C-N>")?;
        self.run("--remote-expr", &open_expr(path, line))
    }

    fn run(&self, flag: &str, arg: &str) -> Result<()> {
        let err = |message: String| Error::Remote {
            addr: self.addr.to_string_lossy().into_owned(),
            message,
        };
        let mut child = Command::new(&self.program)
            .arg("--server")
            .arg(&self.addr)
            .arg(flag)
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(format!("no se pudo ejecutar {:?}: {e}", self.program)))?;
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() >= REMOTE_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err("tiempo de espera agotado (3 s)".into()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(err(e.to_string())),
            }
        };
        if status.success() {
            return Ok(());
        }
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut stderr);
        }
        Err(err(format!("{status}: {}", stderr.trim())))
    }
}

/// Escapa un texto como string Vim entre comillas simples (las `'` se duplican).
fn vim_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Expresión que abre el archivo: `execute(['drop ' .. fnameescape('<ruta>'), '<línea>'])`.
fn open_expr(path: &Path, line: Option<u32>) -> String {
    let drop = format!(
        "'drop ' .. fnameescape({})",
        vim_quote(&path.to_string_lossy())
    );
    match line {
        Some(n) => format!("execute([{drop}, '{n}'])"),
        None => format!("execute({drop})"),
    }
}

/// Escapa una ruta para la línea de comandos de Vim: `\` delante de espacios y `|%#"`
/// (y de `\` fuera de Windows).
fn vim_escape_path(path: &str) -> String {
    let mut out = String::new();
    for c in path.chars() {
        let special = matches!(c, ' ' | '|' | '%' | '#' | '"') || (c == '\\' && cfg!(unix));
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Teclas que la TUI escribe en el panel para abrir un archivo en editores sin canal
/// remoto. **Best effort**: asumen modo normal y sin prompts abiertos; si el editor está
/// en otro estado pueden hacer cosas inesperadas. `Neovim` y `Other` devuelven `None`
/// (nvim usa [`NvimRemote`]).
pub fn open_file_keys(kind: EditorKind, path: &Path, line: Option<u32>) -> Option<Vec<u8>> {
    let p = path.to_string_lossy();
    let mut keys: Vec<u8> = vec![0x1b];
    match kind {
        EditorKind::Vim => {
            keys.extend(format!(":drop {}\r", vim_escape_path(&p)).bytes());
            if let Some(n) = line {
                keys.extend(format!(":{n}\r").bytes());
            }
        }
        EditorKind::Helix => {
            // Helix parte los argumentos por espacios salvo que estén entre comillas.
            if p.contains(char::is_whitespace) && !p.contains('"') {
                keys.extend(format!(":open \"{p}\"\r").bytes());
            } else {
                keys.extend(format!(":open {p}\r").bytes());
            }
            if let Some(n) = line {
                keys.extend(format!(":goto {n}\r").bytes());
            }
        }
        EditorKind::Neovim | EditorKind::Other => return None,
    }
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(s: &str) -> Vec<String> {
        split_command_line(s).unwrap()
    }

    #[test]
    fn splitter_basico() {
        assert_eq!(split("nvim"), ["nvim"]);
        assert_eq!(split("  nvim   -u  x.lua "), ["nvim", "-u", "x.lua"]);
        assert!(split("").is_empty());
        assert!(split("   ").is_empty());
    }

    #[test]
    fn splitter_comillas() {
        assert_eq!(split(r#"nvim -u "a b/x.lua""#), ["nvim", "-u", "a b/x.lua"]);
        assert_eq!(split("nvim -c 'set nu'"), ["nvim", "-c", "set nu"]);
        assert_eq!(split(r#"a"b c"d"#), ["ab cd"]);
        assert_eq!(split(r#"x "" y"#), ["x", "", "y"]);
        assert_eq!(split(r#""it's""#), ["it's"]);
    }

    #[test]
    fn splitter_comillas_sin_cerrar() {
        assert!(matches!(
            split_command_line("nvim 'abc"),
            Err(Error::UnclosedQuote(_))
        ));
        assert!(split_command_line("nvim \"abc").is_err());
    }

    #[test]
    fn editor_vacio_es_error() {
        let e = editor_command("  ", Path::new("/tmp"), "t").unwrap_err();
        assert!(matches!(e, Error::EmptyEditor));
    }

    #[test]
    fn detect_por_nombre() {
        let d = |s: &str| EditorKind::detect(OsStr::new(s));
        assert_eq!(d("nvim"), EditorKind::Neovim);
        assert_eq!(d("/usr/local/bin/NVIM"), EditorKind::Neovim);
        assert_eq!(d("nvim.exe"), EditorKind::Neovim);
        assert_eq!(d(r"C:\Tools\nvim.exe"), EditorKind::Neovim);
        assert_eq!(d("vim"), EditorKind::Vim);
        assert_eq!(d("vi"), EditorKind::Vim);
        assert_eq!(d("hx"), EditorKind::Helix);
        assert_eq!(d("/opt/helix"), EditorKind::Helix);
        assert_eq!(d("nano"), EditorKind::Other);
        assert_eq!(d("code"), EditorKind::Other);
    }

    #[test]
    fn nvim_recibe_listen_y_otros_no() {
        let spec = editor_command("nvim -u 'my x.lua'", Path::new("/tmp"), "tarea").unwrap();
        assert_eq!(spec.kind, EditorKind::Neovim);
        assert_eq!(spec.command.args[0], "-u");
        assert_eq!(spec.command.args[1], "my x.lua");
        assert_eq!(spec.command.args[2], "--listen");
        assert_eq!(spec.remote.unwrap().addr(), spec.command.args[3]);

        let spec = editor_command("vim -n", Path::new("/tmp"), "tarea").unwrap();
        assert!(spec.remote.is_none());
        assert_eq!(spec.command.args, ["-n"]);
    }

    #[test]
    fn escape_de_rutas_en_expresion() {
        assert_eq!(
            open_expr(Path::new("/a/b.txt"), None),
            "execute('drop ' .. fnameescape('/a/b.txt'))"
        );
        assert_eq!(
            open_expr(Path::new("/a/it's here.txt"), Some(50)),
            "execute(['drop ' .. fnameescape('/a/it''s here.txt'), '50'])"
        );
        assert_eq!(vim_quote("ñandú|é"), "'ñandú|é'");
    }

    #[test]
    fn escape_de_rutas_para_teclas_de_vim() {
        #[cfg(unix)]
        assert_eq!(vim_escape_path("/a b/c|d%e#f.txt"), r"/a\ b/c\|d\%e\#f.txt");
        assert_eq!(vim_escape_path("/ñ/é.txt"), "/ñ/é.txt");
    }

    #[test]
    fn direcciones_unicas_y_saneadas() {
        let dir = Path::new("/tmp/qorvi-1");
        let p = socket_path_in(dir, "mi tarea/√", 42);
        assert_eq!(p, Path::new("/tmp/qorvi-1/mi_tarea__-42.sock"));
        assert_eq!(sanitize(""), "ed");
        assert_ne!(socket_path_in(dir, "a", 1), socket_path_in(dir, "a", 2));
    }

    #[test]
    fn direccion_larga_usa_hash() {
        let dir = Path::new("/tmp/qorvi-1");
        let largo = "x".repeat(200);
        let p = socket_path_in(dir, &largo, 42);
        assert!(p.as_os_str().len() <= MAX_SOCKET_PATH);
        assert_eq!(
            p.file_name().unwrap().to_string_lossy(),
            format!("{}-42.sock", short_hash(&largo))
        );
        // El hash distingue instancias largas distintas.
        assert_ne!(p, socket_path_in(dir, &format!("{largo}y"), 42));
        assert_eq!(short_hash(""), "cbf29ce4");
    }

    #[cfg(unix)]
    #[test]
    fn direccion_real_en_directorio_privado() {
        use std::os::unix::fs::PermissionsExt;
        let addr = nvim_address("test-editor", std::process::id()).unwrap();
        let path = PathBuf::from(addr);
        assert!(path.as_os_str().len() <= MAX_SOCKET_PATH);
        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn teclas_para_otros_editores() {
        let p = Path::new("/a b/c.txt");
        #[cfg(unix)]
        assert_eq!(
            open_file_keys(EditorKind::Vim, p, Some(7)).unwrap(),
            b"\x1b:drop /a\\ b/c.txt\r:7\r"
        );
        assert_eq!(
            open_file_keys(EditorKind::Helix, Path::new("/x/y.rs"), Some(3)).unwrap(),
            b"\x1b:open /x/y.rs\r:goto 3\r"
        );
        assert_eq!(
            open_file_keys(EditorKind::Helix, p, None).unwrap(),
            b"\x1b:open \"/a b/c.txt\"\r"
        );
        assert!(open_file_keys(EditorKind::Neovim, p, None).is_none());
        assert!(open_file_keys(EditorKind::Other, p, None).is_none());
    }
}
