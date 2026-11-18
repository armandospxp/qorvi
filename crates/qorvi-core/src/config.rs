//! Configuración TOML en dos capas: global y por repositorio.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::paths::{Paths, RepoPaths};

/// Estilo de atajos (mismos nombres que `QORVI_KEYS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyStyle {
    Auto,
    Direct,
    Prefix,
}

/// Un agente: ejecutable que habla ACP por stdio.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Modo de permisos inicial de la sesión (id de modo del agente, p. ej. `plan`).
    #[serde(default)]
    pub mode: Option<String>,
}

/// Sección `[keys]`: estilo y atajos personalizados (`acción = "tecla"`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct KeysConfig {
    pub style: Option<KeyStyle>,
    #[serde(flatten)]
    pub bindings: BTreeMap<String, String>,
}

/// Contenido crudo de un archivo de config (una capa).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigLayer {
    #[serde(default)]
    pub default_agent: Option<String>,
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub copy_files: Option<Vec<String>>,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub keys: KeysConfig,
}

impl ConfigLayer {
    /// Parsea el texto TOML; `path` solo se usa en el mensaje de error.
    pub fn parse(text: &str, path: &Path) -> Result<ConfigLayer> {
        toml::from_str(text).map_err(|e| Error::Config {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Lee y parsea un archivo; `None` si no existe.
    fn load(path: &Path) -> Result<Option<ConfigLayer>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(path, e)),
        }
    }
}

/// Configuración efectiva tras fusionar global y repo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub default_agent: Option<String>,
    pub editor: Option<String>,
    pub shell: Option<String>,
    pub copy_files: Vec<String>,
    pub setup: Option<String>,
    pub agents: BTreeMap<String, AgentConfig>,
    pub keys: KeysConfig,
}

impl Config {
    /// Carga la capa global y la del repo (ambas opcionales) y las fusiona.
    pub fn load(paths: &Paths, repo: Option<&RepoPaths>) -> Result<Config> {
        let global = ConfigLayer::load(&paths.global_config())?;
        let repo = match repo {
            Some(r) => ConfigLayer::load(&r.repo_config)?,
            None => None,
        };
        Ok(Config::from_layers(global, repo))
    }

    /// Fusiona campo a campo; la capa del repo tiene prioridad.
    pub fn from_layers(global: Option<ConfigLayer>, repo: Option<ConfigLayer>) -> Config {
        let g = global.unwrap_or_default();
        let r = repo.unwrap_or_default();
        let mut agents = g.agents;
        agents.extend(r.agents);
        let mut bindings = g.keys.bindings;
        bindings.extend(r.keys.bindings);
        Config {
            default_agent: r.default_agent.or(g.default_agent),
            editor: r.editor.or(g.editor),
            shell: r.shell.or(g.shell),
            copy_files: r.copy_files.or(g.copy_files).unwrap_or_default(),
            setup: r.setup.or(g.setup),
            agents,
            keys: KeysConfig {
                style: r.keys.style.or(g.keys.style),
                bindings,
            },
        }
    }

    /// Editor: `editor`, si no `$VISUAL`, si no `$EDITOR` (vacíos cuentan como ausentes).
    pub fn editor_command(&self) -> Option<String> {
        let var = |n: &str| std::env::var(n).ok();
        resolve_editor(self.editor.as_deref(), var("VISUAL"), var("EDITOR"))
    }

    /// Shell: `shell`, si no `$SHELL`; en Windows `pwsh`, `powershell` o `cmd`.
    pub fn shell_command(&self) -> String {
        resolve_shell(
            self.shell.as_deref(),
            std::env::var("SHELL").ok(),
            cfg!(windows),
            &in_path,
        )
    }

    /// Agente pedido (o el por defecto) con su nombre.
    pub fn agent(&self, name: Option<&str>) -> Result<(&str, &AgentConfig)> {
        let available = || {
            if self.agents.is_empty() {
                "(ninguno)".to_string()
            } else {
                self.agents.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        };
        let name = name
            .or(self.default_agent.as_deref())
            .ok_or_else(|| Error::NoDefaultAgent {
                available: available(),
            })?;
        match self.agents.get_key_value(name) {
            Some((k, v)) => Ok((k.as_str(), v)),
            None => Err(Error::UnknownAgent {
                name: name.to_string(),
                available: available(),
            }),
        }
    }
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty()).map(str::to_string)
}

fn resolve_editor(
    config: Option<&str>,
    visual: Option<String>,
    editor: Option<String>,
) -> Option<String> {
    non_empty(config)
        .or_else(|| non_empty(visual.as_deref()))
        .or_else(|| non_empty(editor.as_deref()))
}

fn resolve_shell(
    config: Option<&str>,
    shell_env: Option<String>,
    windows: bool,
    in_path: &dyn Fn(&str) -> bool,
) -> String {
    if let Some(s) = non_empty(config) {
        return s;
    }
    if let Some(s) = non_empty(shell_env.as_deref()) {
        return s;
    }
    if windows {
        for candidate in ["pwsh", "powershell"] {
            if in_path(candidate) {
                return candidate.to_string();
            }
        }
        return "cmd".to_string();
    }
    "sh".to_string()
}

/// Búsqueda simple de un ejecutable en `PATH` (con `.exe` en Windows).
fn in_path(exe: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        dir.join(exe).is_file() || (cfg!(windows) && dir.join(format!("{exe}.exe")).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn layer(text: &str) -> ConfigLayer {
        ConfigLayer::parse(text, Path::new("t.toml")).unwrap()
    }

    #[test]
    fn escalares_repo_gana() {
        let g = layer("editor = \"vi\"\nshell = \"zsh\"\nsetup = \"a\"\ndefault_agent = \"x\"");
        let r = layer("editor = \"nvim\"");
        let c = Config::from_layers(Some(g), Some(r));
        assert_eq!(c.editor.as_deref(), Some("nvim"));
        assert_eq!(c.shell.as_deref(), Some("zsh"));
        assert_eq!(c.setup.as_deref(), Some("a"));
        assert_eq!(c.default_agent.as_deref(), Some("x"));
    }

    #[test]
    fn copy_files_reemplaza_solo_si_el_repo_la_define() {
        let g = layer("copy_files = [\".env\"]");
        let c = Config::from_layers(Some(g.clone()), Some(layer("")));
        assert_eq!(c.copy_files, vec![".env"]);
        let c = Config::from_layers(Some(g.clone()), Some(layer("copy_files = []")));
        assert!(c.copy_files.is_empty());
        let c = Config::from_layers(Some(g), Some(layer("copy_files = [\"a\"]")));
        assert_eq!(c.copy_files, vec!["a"]);
    }

    #[test]
    fn agentes_y_atajos_se_fusionan_por_clave() {
        let g = layer(
            "[agents.a]\ncommand = \"ga\"\n[agents.b]\ncommand = \"gb\"\n\
             [keys]\nstyle = \"direct\"\nfocus_left = \"alt-h\"\nfocus_right = \"alt-l\"",
        );
        let r = layer(
            "[agents.b]\ncommand = \"rb\"\nargs = [\"-x\"]\n[agents.c]\ncommand = \"rc\"\nmode = \"plan\"\n\
             [keys]\nfocus_left = \"ctrl-h\"",
        );
        let c = Config::from_layers(Some(g), Some(r));
        assert_eq!(c.agents["a"].command, "ga");
        assert_eq!(c.agents["b"].command, "rb");
        assert_eq!(c.agents["b"].args, vec!["-x"]);
        assert_eq!(c.agents["c"].command, "rc");
        assert_eq!(c.agents["c"].mode.as_deref(), Some("plan"));
        assert_eq!(c.agents["a"].mode, None);
        assert_eq!(c.keys.style, Some(KeyStyle::Direct));
        assert_eq!(c.keys.bindings["focus_left"], "ctrl-h");
        assert_eq!(c.keys.bindings["focus_right"], "alt-l");
    }

    #[test]
    fn sin_capas_da_valores_vacios() {
        assert_eq!(Config::from_layers(None, None), Config::default());
    }

    #[test]
    fn toml_invalido_incluye_ruta_y_posicion() {
        let e = ConfigLayer::parse("editor = \n", Path::new("/x/config.toml")).unwrap_err();
        let msg = e.to_string();
        assert!(matches!(e, Error::Config { .. }));
        assert!(msg.contains("/x/config.toml"), "{msg}");
        assert!(msg.contains("line 1"), "{msg}");
    }

    #[test]
    fn campo_desconocido_rechazado() {
        let e = ConfigLayer::parse("editr = \"x\"", Path::new("c.toml")).unwrap_err();
        assert!(e.to_string().contains("editr"), "{e}");
        let e = ConfigLayer::parse(
            "[agents.a]\ncommand = \"x\"\nargz = []",
            Path::new("c.toml"),
        )
        .unwrap_err();
        assert!(e.to_string().contains("argz"), "{e}");
    }

    #[test]
    fn estilo_invalido_rechazado() {
        assert!(ConfigLayer::parse("[keys]\nstyle = \"raro\"", Path::new("c.toml")).is_err());
    }

    #[test]
    fn archivos_inexistentes_no_son_error() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("c"), d.path().join("d"));
        let c = Config::load(&paths, None).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn load_lee_ambas_capas() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::new(d.path().join("c"), d.path().join("d"));
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.global_config(), "editor = \"vi\"\nshell = \"zsh\"").unwrap();
        let rp = RepoPaths {
            repo_config: d.path().join(".qorvi.toml"),
            state_dir: PathBuf::new(),
            tasks_file: PathBuf::new(),
            worktrees_dir: PathBuf::new(),
        };
        std::fs::write(&rp.repo_config, "editor = \"nvim\"").unwrap();
        let c = Config::load(&paths, Some(&rp)).unwrap();
        assert_eq!(c.editor.as_deref(), Some("nvim"));
        assert_eq!(c.shell.as_deref(), Some("zsh"));
        std::fs::write(&rp.repo_config, "editor = ").unwrap();
        let e = Config::load(&paths, Some(&rp)).unwrap_err();
        assert!(e.to_string().contains(".qorvi.toml"), "{e}");
    }

    #[test]
    fn ejemplo_parsea() {
        let l = layer(include_str!("../../../examples/config.example.toml"));
        assert_eq!(l.default_agent.as_deref(), Some("mi-agente"));
        assert_eq!(l.copy_files, Some(vec![".env".into(), ".env.local".into()]));
        assert_eq!(l.agents["mi-agente"].command, "mi-agente-acp");
        let c = Config::from_layers(Some(l), None);
        assert!(c.agent(None).is_ok());
    }

    #[test]
    fn editor_resolucion() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(resolve_editor(Some("hx"), s("v"), s("e")), s("hx"));
        assert_eq!(resolve_editor(None, s("v"), s("e")), s("v"));
        assert_eq!(resolve_editor(None, s(""), s("e")), s("e"));
        assert_eq!(resolve_editor(Some(""), None, s("e")), s("e"));
        assert_eq!(resolve_editor(None, None, s("")), None);
    }

    #[test]
    fn shell_resolucion() {
        let s = |v: &str| Some(v.to_string());
        let todo = |_: &str| true;
        let nada = |_: &str| false;
        let solo_ps = |e: &str| e == "powershell";
        assert_eq!(
            resolve_shell(Some("fish"), s("/bin/zsh"), false, &nada),
            "fish"
        );
        assert_eq!(resolve_shell(None, s("/bin/zsh"), false, &nada), "/bin/zsh");
        assert_eq!(resolve_shell(None, s(""), true, &todo), "pwsh");
        assert_eq!(resolve_shell(None, None, true, &solo_ps), "powershell");
        assert_eq!(resolve_shell(None, None, true, &nada), "cmd");
        assert_eq!(resolve_shell(None, None, false, &nada), "sh");
    }

    #[test]
    fn agente_resolucion() {
        let c = Config::from_layers(
            Some(layer(
                "default_agent = \"a\"\n[agents.a]\ncommand = \"x\"\n[agents.b]\ncommand = \"y\"",
            )),
            None,
        );
        assert_eq!(c.agent(None).unwrap().0, "a");
        assert_eq!(c.agent(Some("b")).unwrap().1.command, "y");
        let e = c.agent(Some("zz")).unwrap_err();
        assert!(matches!(e, Error::UnknownAgent { .. }));
        assert!(e.to_string().contains("a, b"), "{e}");
        let e = Config::default().agent(None).unwrap_err();
        assert!(matches!(e, Error::NoDefaultAgent { .. }));
        let c2 = Config::from_layers(Some(layer("default_agent = \"q\"")), None);
        assert!(matches!(c2.agent(None), Err(Error::UnknownAgent { .. })));
    }
}
