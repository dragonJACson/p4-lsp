//! Configuration file support for p4-lsp.
//!
//! This module handles parsing of `.p4lsp.json` configuration files,
//! which provide explicit project structure for complex P4 projects.
//!
//! # Example `.p4lsp.json`
//!
//! ```json
//! {
//!   "version": 1,
//!   "includePaths": ["include", "/opt/p4/share/p4include"],
//!   "targets": [
//!     {
//!       "name": "pipeline_tx",
//!       "root": "pipeline-tx/pipeline_tx.p4"
//!     },
//!     {
//!       "name": "pipeline_rx",
//!       "root": "pipeline-rx/pipeline_rx.p4"
//!     }
//!   ]
//! }
//! ```

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The name of the configuration file.
pub const CONFIG_FILE_NAME: &str = ".p4lsp.json";

/// Root configuration structure for `.p4lsp.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    /// Config file version (currently 1).
    #[serde(default = "default_version")]
    pub version: u32,

    /// Additional include paths for resolving `#include` directives.
    /// Paths are relative to the workspace root or absolute.
    #[serde(default)]
    pub include_paths: Vec<String>,

    /// Explicit target definitions. Each target represents a "root" P4 file
    /// that should be compiled as a complete program.
    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

fn default_version() -> u32 {
    1
}

/// A compilation target definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetConfig {
    /// Human-readable name for this target (optional, for display).
    #[serde(default)]
    pub name: String,

    /// Path to the root P4 file for this target.
    /// Relative paths are resolved from the workspace root.
    pub root: String,

    /// Optional target-specific include paths (in addition to global ones).
    #[serde(default)]
    pub include_paths: Vec<String>,
}

/// Resolved configuration with absolute paths.
#[derive(Debug, Clone, Default)]
pub struct ResolvedConfig {
    /// Absolute paths to include directories.
    pub include_paths: Vec<PathBuf>,

    /// Resolved target definitions with absolute paths.
    pub targets: Vec<ResolvedTarget>,
}

/// A resolved target with absolute path.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub name: String,
    pub root: PathBuf,
    pub include_paths: Vec<PathBuf>,
}

impl ProjectConfig {
    /// Load configuration from a directory (looks for `.p4lsp.json`).
    pub fn load_from_dir(dir: &Path) -> Option<Self> {
        let config_path = dir.join(CONFIG_FILE_NAME);
        debug!("Looking for config file: {}", config_path.display());
        Self::load_from_file(&config_path)
    }

    /// Load configuration from a specific file path.
    pub fn load_from_file(path: &Path) -> Option<Self> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                debug!("No config file at {}: {}", path.display(), e);
                return None;
            }
        };
        match serde_json::from_str(&content) {
            Ok(config) => {
                info!("Loaded config from {}", path.display());
                Some(config)
            }
            Err(e) => {
                warn!("Failed to parse {}: {}", path.display(), e);
                None
            }
        }
    }

    /// Resolve all relative paths to absolute paths based on workspace root.
    pub fn resolve(&self, workspace_root: &Path) -> ResolvedConfig {
        let include_paths = self
            .include_paths
            .iter()
            .map(|p| resolve_path(workspace_root, p))
            .collect();

        let targets = self
            .targets
            .iter()
            .map(|t| ResolvedTarget {
                name: if t.name.is_empty() {
                    Path::new(&t.root)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unnamed")
                        .to_string()
                } else {
                    t.name.clone()
                },
                root: resolve_path(workspace_root, &t.root),
                include_paths: t
                    .include_paths
                    .iter()
                    .map(|p| resolve_path(workspace_root, p))
                    .collect(),
            })
            .collect();

        ResolvedConfig {
            include_paths,
            targets,
        }
    }
}

fn resolve_path(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_minimal_config() {
        let json = r#"{"version": 1}"#;
        let config: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, 1);
        assert!(config.include_paths.is_empty());
        assert!(config.targets.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let json = r#"{
            "version": 1,
            "includePaths": ["include", "/opt/p4"],
            "targets": [
                {
                    "name": "tx_pipeline",
                    "root": "pipeline-tx/pipeline_tx.p4"
                },
                {
                    "root": "pipeline-rx/pipeline_rx.p4",
                    "includePaths": ["pipeline-rx/include"]
                }
            ]
        }"#;
        let config: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.version, 1);
        assert_eq!(config.include_paths, vec!["include", "/opt/p4"]);
        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.targets[0].name, "tx_pipeline");
        assert_eq!(config.targets[0].root, "pipeline-tx/pipeline_tx.p4");
        assert_eq!(config.targets[1].name, "");
        assert_eq!(config.targets[1].include_paths, vec!["pipeline-rx/include"]);
    }

    #[test]
    fn test_resolve_paths() {
        let json = r#"{
            "version": 1,
            "includePaths": ["include", "/opt/p4"],
            "targets": [
                {"root": "src/main.p4"},
                {"name": "other", "root": "/absolute/path.p4"}
            ]
        }"#;
        let config: ProjectConfig = serde_json::from_str(json).unwrap();
        let workspace = Path::new("/home/user/project");
        let resolved = config.resolve(workspace);

        assert_eq!(
            resolved.include_paths,
            vec![
                PathBuf::from("/home/user/project/include"),
                PathBuf::from("/opt/p4"),
            ]
        );
        assert_eq!(resolved.targets.len(), 2);
        assert_eq!(resolved.targets[0].name, "main");
        assert_eq!(
            resolved.targets[0].root,
            PathBuf::from("/home/user/project/src/main.p4")
        );
        assert_eq!(resolved.targets[1].name, "other");
        assert_eq!(resolved.targets[1].root, PathBuf::from("/absolute/path.p4"));
    }

    #[test]
    fn test_load_from_file() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join(".p4lsp.json");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(
            file,
            r#"{{"version": 1, "targets": [{{"root": "main.p4"}}]}}"#
        )
        .unwrap();

        let config = ProjectConfig::load_from_dir(dir.path()).unwrap();
        assert_eq!(config.version, 1);
        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.targets[0].root, "main.p4");
    }

    #[test]
    fn test_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let config = ProjectConfig::load_from_dir(dir.path());
        assert!(config.is_none());
    }

    #[test]
    fn test_camel_case_parsing() {
        // Ensure camelCase is used (not snake_case)
        let json = r#"{
            "includePaths": ["a"],
            "targets": [{"root": "b.p4", "includePaths": ["c"]}]
        }"#;
        let config: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.include_paths, vec!["a"]);
        assert_eq!(config.targets[0].include_paths, vec!["c"]);
    }
}
