//! Filas visibles del explorador de archivos: función pura, sin IO.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::{ChangeMark, FileEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Dir { expanded: bool, changed: bool },
    File { mark: Option<ChangeMark> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Ruta completa relativa, sin `/` final.
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub kind: RowKind,
}

impl Row {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, RowKind::Dir { .. })
    }

    /// Ruta de la carpeta que lo contiene, si no está en la raíz.
    pub fn parent_path(&self) -> Option<&str> {
        self.path.rsplit_once('/').map(|(parent, _)| parent)
    }
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: BTreeMap<String, Option<ChangeMark>>,
    changed: bool,
}

fn build_tree(files: &[FileEntry]) -> Node {
    let mut root = Node::default();
    for f in files {
        let mut parts: Vec<&str> = f.path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        let mut node = &mut root;
        for dir in parts {
            node = node.dirs.entry(dir.to_string()).or_default();
            node.changed |= f.mark.is_some();
        }
        node.files.insert(name.to_string(), f.mark);
    }
    root
}

fn flatten(
    node: &Node,
    prefix: &str,
    depth: usize,
    expanded: &BTreeSet<String>,
    out: &mut Vec<Row>,
) {
    for (name, dir) in &node.dirs {
        let path = format!("{prefix}{name}");
        let is_open = expanded.contains(&path);
        out.push(Row {
            path: path.clone(),
            name: name.clone(),
            depth,
            kind: RowKind::Dir {
                expanded: is_open,
                changed: dir.changed,
            },
        });
        if is_open {
            flatten(dir, &format!("{path}/"), depth + 1, expanded, out);
        }
    }
    for (name, mark) in &node.files {
        out.push(Row {
            path: format!("{prefix}{name}"),
            name: name.clone(),
            depth,
            kind: RowKind::File { mark: *mark },
        });
    }
}

/// Filas a mostrar: carpetas antes que archivos, orden alfabético (por bytes) y los
/// hijos de las carpetas contraídas ocultos.
pub fn visible_rows(files: &[FileEntry], expanded: &BTreeSet<String>) -> Vec<Row> {
    let mut out = Vec::new();
    flatten(&build_tree(files), "", 0, expanded, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, mark: Option<ChangeMark>) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            mark,
        }
    }

    fn open(dirs: &[&str]) -> BTreeSet<String> {
        dirs.iter().map(|d| d.to_string()).collect()
    }

    fn paths(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.path.as_str()).collect()
    }

    fn sample() -> Vec<FileEntry> {
        vec![
            entry("src/lib.rs", Some(ChangeMark::Modified)),
            entry("README.md", None),
            entry("Cargo.toml", None),
            entry("src/session/store.rs", None),
            entry("src/session/mod.rs", Some(ChangeMark::Added)),
            entry("src/main.rs", None),
            entry(".github/workflows/ci.yml", None),
            entry("tests/integration.rs", None),
        ]
    }

    #[test]
    fn dirs_first_then_files_alphabetical() {
        let rows = visible_rows(&sample(), &open(&[]));
        assert_eq!(
            paths(&rows),
            [".github", "src", "tests", "Cargo.toml", "README.md"]
        );
    }

    #[test]
    fn nesting_and_depth() {
        let rows = visible_rows(&sample(), &open(&["src", "src/session"]));
        assert_eq!(
            paths(&rows),
            [
                ".github",
                "src",
                "src/session",
                "src/session/mod.rs",
                "src/session/store.rs",
                "src/lib.rs",
                "src/main.rs",
                "tests",
                "Cargo.toml",
                "README.md"
            ]
        );
        assert_eq!(rows[1].depth, 0);
        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[3].depth, 2);
        assert_eq!(rows[3].name, "mod.rs");
        assert_eq!(rows[3].parent_path(), Some("src/session"));
        assert_eq!(rows[8].parent_path(), None);
    }

    #[test]
    fn collapsed_dir_hides_children_even_if_inner_is_expanded() {
        let rows = visible_rows(&sample(), &open(&["src/session"]));
        assert!(!paths(&rows).iter().any(|p| p.starts_with("src/")));
    }

    #[test]
    fn changed_propagates_to_all_ancestors() {
        let rows = visible_rows(&sample(), &open(&["src"]));
        let kind = |p: &str| rows.iter().find(|r| r.path == p).map(|r| r.kind);
        assert_eq!(
            kind("src"),
            Some(RowKind::Dir {
                expanded: true,
                changed: true
            })
        );
        assert_eq!(
            kind("src/session"),
            Some(RowKind::Dir {
                expanded: false,
                changed: true
            })
        );
        assert_eq!(
            kind(".github"),
            Some(RowKind::Dir {
                expanded: false,
                changed: false
            })
        );
    }

    #[test]
    fn empty_tree_has_no_rows() {
        assert!(visible_rows(&[], &open(&[])).is_empty());
    }
}
