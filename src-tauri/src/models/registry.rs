//! Loads manifests from disk and answers "what can this machine actually run?".

use super::manifest::{Manifest, ManifestError};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Registry {
    models: BTreeMap<String, Manifest>,
    /// Files that failed to load, kept so the UI can show them instead of hiding a typo.
    problems: Vec<LoadProblem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadProblem {
    pub path: PathBuf,
    pub message: String,
}

impl Registry {
    /// Scans every `*.toml` directly inside each directory, later directories winning on
    /// id collision. Order is: bundled manifests, then the user's own directory - so a
    /// user can shadow a shipped model with their own tweaked copy.
    pub fn load_dirs(dirs: &[PathBuf]) -> Self {
        let mut registry = Registry::default();
        for dir in dirs {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => continue, // a missing user dir is normal, not a problem
            };
            let mut paths: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
                .collect();
            paths.sort();
            let mut here = BTreeMap::new();
            for path in paths {
                registry.load_one(&path, &mut here);
            }
        }
        registry
    }

    /// `here` is the ids already loaded from this same directory. A later directory
    /// shadowing an earlier one is the point; two files in one directory with one id is a
    /// copy someone forgot to rename, and silently using whichever sorts last hides it.
    fn load_one(&mut self, path: &Path, here: &mut BTreeMap<String, PathBuf>) {
        match Manifest::load(path) {
            Ok(manifest) => {
                if let Some(first) = here.insert(manifest.id.clone(), path.to_path_buf()) {
                    self.problems.push(LoadProblem {
                        path: path.to_path_buf(),
                        message: format!(
                            "id {:?} is also used by {}; this file replaces it",
                            manifest.id,
                            first.display()
                        ),
                    });
                }
                self.models.insert(manifest.id.clone(), manifest);
            }
            Err(err) => self.problems.push(LoadProblem {
                path: path.to_path_buf(),
                message: describe(&err),
            }),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Manifest> {
        self.models.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Manifest> {
        self.models.values()
    }

    pub fn problems(&self) -> &[LoadProblem] {
        &self.problems
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

fn describe(err: &ManifestError) -> String {
    // toml's parse errors are multi-line with a source snippet; keep it, it is the most
    // useful thing we can show someone writing a manifest.
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, name: &str) -> String {
        format!("schema = 1\nid = {id:?}\nname = {name:?}\ntask = \"custom\"\n[[run]]\ncommand = \"true\"\n")
    }

    #[test]
    fn later_directories_shadow_and_problems_are_kept() {
        let root = std::env::temp_dir().join(format!("geis-registry-{}", uuid::Uuid::new_v4()));
        let bundled = root.join("bundled");
        let user = root.join("user");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(bundled.join("a.toml"), manifest("a", "Shipped")).unwrap();
        std::fs::write(bundled.join("broken.toml"), "schema = ").unwrap();
        std::fs::write(bundled.join("notes.txt"), "not a manifest").unwrap();
        std::fs::write(user.join("a.toml"), manifest("a", "Mine")).unwrap();
        std::fs::write(user.join("b.toml"), manifest("b", "B")).unwrap();
        std::fs::write(user.join("b copy.toml"), manifest("b", "B copy")).unwrap();

        let registry = Registry::load_dirs(&[bundled.clone(), user.clone(), root.join("missing")]);
        assert_eq!(registry.len(), 2);
        assert_eq!(
            registry.get("a").unwrap().name,
            "Mine",
            "the user's copy wins"
        );
        let problems: Vec<_> = registry.problems().iter().map(|p| p.path.clone()).collect();
        assert!(problems.contains(&bundled.join("broken.toml")));
        assert!(
            problems.contains(&user.join("b.toml")),
            "the duplicate id is reported"
        );
        assert!(
            !problems.iter().any(|p| p.ends_with("a.toml")),
            "shadowing is not a problem"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
