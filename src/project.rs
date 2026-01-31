use dashmap::DashMap;
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use p4::ast::AST;
use p4::check::{self, Diagnostics};

use p4::lexer::Lexer;
use p4::parser::Parser;
use p4::preprocessor::MacroEnv;

use crate::config::{ProjectConfig, ResolvedConfig};

pub type FilePath = Arc<String>;

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub def_file: FilePath,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Struct,
    Header,
    Typedef,
    Constant,
    Action,
    Control,
    Parser,
    Table,
    Extern,
    Package,
    Enum,
    EnumMember,
    Variable,
    Parameter,
    Macro,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    pub symbols: Vec<Symbol>,
    pub by_name: HashMap<String, Vec<usize>>,
    pub by_file: HashMap<FilePath, Vec<usize>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, symbol: Symbol) {
        let idx = self.symbols.len();
        self.by_name
            .entry(symbol.name.clone())
            .or_default()
            .push(idx);
        self.by_file
            .entry(symbol.def_file.clone())
            .or_default()
            .push(idx);
        self.symbols.push(symbol);
    }

    pub fn lookup(&self, name: &str) -> Vec<&Symbol> {
        self.by_name
            .get(name)
            .map(|ids| ids.iter().filter_map(|&i| self.symbols.get(i)).collect())
            .unwrap_or_default()
    }

    pub fn symbols_in_file(&self, file: &FilePath) -> Vec<&Symbol> {
        self.by_file
            .get(file)
            .map(|ids| ids.iter().filter_map(|&i| self.symbols.get(i)).collect())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct FileSnapshot {
    pub version: i32,
    pub text: String,
}

/// Result of compiling a root file.
#[derive(Debug)]
pub struct RootUnit {
    pub root: FilePath,
    pub include_order: Vec<FilePath>,
    pub include_index: HashMap<FilePath, usize>,
    pub symbols: SymbolTable,
    pub macro_env: MacroEnv,
    pub ast: AST,
    pub diagnostics: Diagnostics,
    /// Warnings collected during compilation (e.g., unresolved includes).
    pub warnings: Vec<String>,
}

impl RootUnit {
    pub fn symbols_visible_at(&self, file: &FilePath) -> Vec<&Symbol> {
        let pos = self.include_index.get(file).copied().unwrap_or(0);
        let mut result = Vec::new();
        for i in 0..=pos {
            if let Some(f) = self.include_order.get(i) {
                result.extend(self.symbols.symbols_in_file(f));
            }
        }
        result
    }
}

pub struct ProjectState {
    pub workspace_root: Option<PathBuf>,
    pub vfs: DashMap<FilePath, FileSnapshot>,
    pub roots: DashMap<FilePath, RootUnit>,
    pub file_to_roots: DashMap<FilePath, HashSet<FilePath>>,
    pub include_paths: Vec<PathBuf>,
    pub config: Option<ResolvedConfig>,
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            workspace_root: None,
            vfs: DashMap::new(),
            roots: DashMap::new(),
            file_to_roots: DashMap::new(),
            include_paths: Vec::new(),
            config: None,
        }
    }

    pub fn set_workspace_root(&mut self, root: PathBuf) {
        info!("Setting workspace root: {}", root.display());

        // Try to load config from workspace root
        if let Some(config) = ProjectConfig::load_from_dir(&root) {
            let resolved = config.resolve(&root);
            info!(
                "Config loaded: {} targets, {} global include paths",
                resolved.targets.len(),
                resolved.include_paths.len()
            );
            for target in &resolved.targets {
                info!("  Target '{}': {}", target.name, target.root.display());
            }
            for path in &resolved.include_paths {
                info!("  Include path: {}", path.display());
                if !self.include_paths.contains(path) {
                    self.include_paths.push(path.clone());
                }
            }
            self.config = Some(resolved);
        } else {
            info!("No .p4lsp.json found, using auto-discovery");
        }
        self.workspace_root = Some(root);
    }

    pub fn add_include_path(&mut self, path: PathBuf) {
        if !self.include_paths.contains(&path) {
            self.include_paths.push(path);
        }
    }

    /// Returns true if explicit targets are configured.
    pub fn has_explicit_targets(&self) -> bool {
        self.config
            .as_ref()
            .map(|c| !c.targets.is_empty())
            .unwrap_or(false)
    }

    /// Get explicit root paths from config, or None if auto-discovery should be used.
    pub fn explicit_roots(&self) -> Option<Vec<FilePath>> {
        let config = self.config.as_ref()?;
        if config.targets.is_empty() {
            return None;
        }
        Some(
            config
                .targets
                .iter()
                .filter(|t| t.root.exists())
                .map(|t| Arc::new(t.root.to_string_lossy().to_string()))
                .collect(),
        )
    }

    /// Get include paths for a specific root file (combines global + target-specific).
    pub fn include_paths_for_root(&self, root: &FilePath) -> Vec<PathBuf> {
        let mut paths = self.include_paths.clone();

        if let Some(config) = &self.config {
            for target in &config.targets {
                if target.root.to_string_lossy() == root.as_str() {
                    for p in &target.include_paths {
                        if !paths.contains(p) {
                            paths.push(p.clone());
                        }
                    }
                    break;
                }
            }
        }

        paths
    }

    pub fn read_file(&self, path: &FilePath) -> Option<String> {
        if let Some(snapshot) = self.vfs.get(path) {
            return Some(snapshot.text.clone());
        }
        fs::read_to_string(path.as_str()).ok()
    }

    pub fn update_file(&self, path: FilePath, version: i32, text: String) {
        self.vfs.insert(path, FileSnapshot { version, text });
    }

    pub fn remove_file(&self, path: &FilePath) {
        self.vfs.remove(path);
    }

    pub fn discover_roots(&self) -> Vec<FilePath> {
        // If explicit targets are configured, use those instead of auto-discovery
        if let Some(roots) = self.explicit_roots() {
            info!(
                "Using {} explicit targets from config",
                roots.len()
            );
            for root in &roots {
                info!("  Root: {}", root);
            }
            return roots;
        }

        let Some(ref workspace) = self.workspace_root else {
            warn!("No workspace root set, cannot discover roots");
            return Vec::new();
        };

        info!("Auto-discovering roots in {}", workspace.display());

        let mut all_files: HashSet<FilePath> = HashSet::new();
        let mut included_files: HashSet<FilePath> = HashSet::new();

        Self::collect_p4_files(workspace, &mut all_files);
        debug!("Found {} P4 files total", all_files.len());

        for file in &all_files {
            if let Some(text) = self.read_file(file) {
                if let Ok(ppr) = p4::preprocessor::run(&text, file.clone()) {
                    for inc in &ppr.elements.includes {
                        let inc_path = self.resolve_include(file, inc);
                        if let Some(resolved) = inc_path {
                            included_files.insert(resolved);
                        }
                    }
                }
            }
        }

        let roots: Vec<FilePath> = all_files.difference(&included_files).cloned().collect();
        info!("Discovered {} root files (not included by others)", roots.len());
        for root in &roots {
            info!("  Root: {}", root);
        }
        roots
    }

    fn collect_p4_files(dir: &Path, files: &mut HashSet<FilePath>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_p4_files(&path, files);
            } else if let Some(ext) = path.extension() {
                if ext == "p4" {
                    files.insert(Arc::new(path.to_string_lossy().to_string()));
                }
            }
        }
    }

    /// Resolve an include using the given include paths.
    pub fn resolve_include_with_paths(
        from_file: &FilePath,
        include: &str,
        include_paths: &[PathBuf],
    ) -> Option<FilePath> {
        let from_path = Path::new(from_file.as_str());
        let parent = from_path.parent().unwrap_or(Path::new("."));

        // Try relative to current file first
        let relative = parent.join(include);
        if relative.exists() {
            let resolved = relative.canonicalize().ok()?.to_string_lossy().to_string();
            debug!("Resolved include '{}' -> {} (relative)", include, resolved);
            return Some(Arc::new(resolved));
        }

        // Try include paths
        for inc_path in include_paths {
            let candidate = inc_path.join(include);
            if candidate.exists() {
                let resolved = candidate.canonicalize().ok()?.to_string_lossy().to_string();
                debug!(
                    "Resolved include '{}' -> {} (via {})",
                    include,
                    resolved,
                    inc_path.display()
                );
                return Some(Arc::new(resolved));
            }
        }

        warn!(
            "Failed to resolve include '{}' from {}",
            include,
            from_file
        );
        None
    }

    /// Resolve an include using global include paths (for auto-discovery).
    pub fn resolve_include(&self, from_file: &FilePath, include: &str) -> Option<FilePath> {
        Self::resolve_include_with_paths(from_file, include, &self.include_paths)
    }

    pub fn compile_root(&self, root_path: &FilePath) -> Option<RootUnit> {
        info!("Compiling root: {}", root_path);

        let text = match self.read_file(root_path) {
            Some(t) => t,
            None => {
                warn!("Cannot read root file: {}", root_path);
                return None;
            }
        };

        // Get effective include paths for this root (global + target-specific)
        let include_paths = self.include_paths_for_root(root_path);
        debug!(
            "Using {} include paths for root {}",
            include_paths.len(),
            root_path
        );
        for p in &include_paths {
            debug!("  Include path: {}", p.display());
        }

        let mut ast = AST::default();
        let mut symbols = SymbolTable::new();
        let mut include_order = Vec::new();
        let mut include_index = HashMap::new();
        let mut visited = HashSet::new();
        let mut warnings = Vec::new();

        let env = self.process_file_recursive(
            root_path,
            &text,
            &MacroEnv::default(),
            &include_paths,
            &mut ast,
            &mut symbols,
            &mut include_order,
            &mut include_index,
            &mut visited,
            &mut warnings,
        );

        if env.is_none() {
            warn!("Failed to process root: {}", root_path);
            return None;
        }

        let (_, diagnostics) = check::all(&ast);

        info!(
            "Compiled root {} with {} files, {} symbols",
            root_path,
            include_order.len(),
            symbols.symbols.len()
        );
        for (i, file) in include_order.iter().enumerate() {
            debug!("  [{}] {}", i, file);
        }

        Some(RootUnit {
            root: root_path.clone(),
            include_order,
            include_index,
            symbols,
            macro_env: env.unwrap(),
            ast,
            diagnostics,
            warnings,
        })
    }

    fn process_file_recursive(
        &self,
        file_path: &FilePath,
        text: &str,
        env_in: &MacroEnv,
        include_paths: &[PathBuf],
        ast: &mut AST,
        symbols: &mut SymbolTable,
        include_order: &mut Vec<FilePath>,
        include_index: &mut HashMap<FilePath, usize>,
        visited: &mut HashSet<FilePath>,
        warnings: &mut Vec<String>,
    ) -> Option<MacroEnv> {
        if visited.contains(file_path) {
            return Some(env_in.clone());
        }
        visited.insert(file_path.clone());

        let ppr = match p4::preprocessor::run_with_env(text, file_path.clone(), env_in) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("Preprocessor error in {}: {:?}", file_path, e);
                warn!("{}", msg);
                warnings.push(msg);
                return None;
            }
        };
        let mut current_env = ppr.env_out.clone();

        for inc in &ppr.elements.includes {
            match Self::resolve_include_with_paths(file_path, inc, include_paths) {
                Some(resolved) => {
                    if let Some(inc_text) = self.read_file(&resolved) {
                        current_env = self.process_file_recursive(
                            &resolved,
                            &inc_text,
                            &current_env,
                            include_paths,
                            ast,
                            symbols,
                            include_order,
                            include_index,
                            visited,
                            warnings,
                        )?;
                    }
                }
                None => {
                    let msg = format!("Cannot resolve include '{}' from {}", inc, file_path);
                    warnings.push(msg);
                }
            }
        }

        include_index.insert(file_path.clone(), include_order.len());
        include_order.push(file_path.clone());

        let lexer = Lexer::new(
            ppr.lines.iter().map(|s| s.as_str()).collect(),
            file_path.clone(),
        );
        let mut parser = Parser::new(lexer);
        if let Err(e) = parser.run(ast) {
            let msg = format!("Parser error in {}: {:?}", file_path, e);
            warn!("{}", msg);
            warnings.push(msg);
            return None;
        }

        self.extract_symbols_from_ast(ast, file_path, symbols);

        Some(current_env)
    }

    fn extract_symbols_from_ast(&self, ast: &AST, file_path: &FilePath, symbols: &mut SymbolTable) {
        for s in &ast.structs {
            if s.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: s.name.clone(),
                    kind: SymbolKind::Struct,
                    def_file: file_path.clone(),
                    line: s.token.line,
                    col: s.token.col,
                });
            }
        }

        for h in &ast.headers {
            if h.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: h.name.clone(),
                    kind: SymbolKind::Header,
                    def_file: file_path.clone(),
                    line: h.token.line,
                    col: h.token.col,
                });
            }
        }

        for t in &ast.typedefs {
            if t.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: t.name.clone(),
                    kind: SymbolKind::Typedef,
                    def_file: file_path.clone(),
                    line: t.token.line,
                    col: t.token.col,
                });
            }
        }

        for c in &ast.constants {
            if c.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: c.name.clone(),
                    kind: SymbolKind::Constant,
                    def_file: file_path.clone(),
                    line: c.token.line,
                    col: c.token.col,
                });
            }
        }

        for ctrl in &ast.controls {
            if ctrl.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: ctrl.name.clone(),
                    kind: SymbolKind::Control,
                    def_file: file_path.clone(),
                    line: ctrl.token.line,
                    col: ctrl.token.col,
                });

                for action in &ctrl.actions {
                    symbols.add(Symbol {
                        name: action.name.clone(),
                        kind: SymbolKind::Action,
                        def_file: file_path.clone(),
                        line: action.token.line,
                        col: action.token.col,
                    });
                }

                for table in &ctrl.tables {
                    symbols.add(Symbol {
                        name: table.name.clone(),
                        kind: SymbolKind::Table,
                        def_file: file_path.clone(),
                        line: table.token.line,
                        col: table.token.col,
                    });
                }
            }
        }

        for p in &ast.parsers {
            if p.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: p.name.clone(),
                    kind: SymbolKind::Parser,
                    def_file: file_path.clone(),
                    line: p.token.line,
                    col: p.token.col,
                });
            }
        }

        for e in &ast.externs {
            if e.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: e.name.clone(),
                    kind: SymbolKind::Extern,
                    def_file: file_path.clone(),
                    line: e.token.line,
                    col: e.token.col,
                });
            }
        }

        for pkg in &ast.packages {
            if pkg.token.file.as_str() == file_path.as_str() {
                symbols.add(Symbol {
                    name: pkg.name.clone(),
                    kind: SymbolKind::Package,
                    def_file: file_path.clone(),
                    line: pkg.token.line,
                    col: pkg.token.col,
                });
            }
        }
    }

    pub fn find_root_for_file(&self, file: &FilePath) -> Option<FilePath> {
        if self.roots.contains_key(file) {
            return Some(file.clone());
        }

        if let Some(roots) = self.file_to_roots.get(file) {
            return roots.iter().next().cloned();
        }

        None
    }

    pub fn rebuild_all_roots(&self) {
        let discovered = self.discover_roots();

        for root_path in discovered {
            if let Some(unit) = self.compile_root(&root_path) {
                for file in &unit.include_order {
                    self.file_to_roots
                        .entry(file.clone())
                        .or_default()
                        .insert(root_path.clone());
                }
                self.roots.insert(root_path, unit);
            }
        }
    }

    pub fn rebuild_root(&self, root_path: &FilePath) {
        if let Some(unit) = self.compile_root(root_path) {
            for file in &unit.include_order {
                self.file_to_roots
                    .entry(file.clone())
                    .or_default()
                    .insert(root_path.clone());
            }
            self.roots.insert(root_path.clone(), unit);
        }
    }

    pub fn invalidate_file(&self, file: &FilePath) {
        let roots_to_rebuild: Vec<FilePath> = self
            .file_to_roots
            .get(file)
            .map(|r| r.iter().cloned().collect())
            .unwrap_or_default();

        for root in roots_to_rebuild {
            self.rebuild_root(&root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_project_with_config() {
        let dir = TempDir::new().unwrap();

        // Create directory structure:
        // workspace/
        //   .p4lsp.json
        //   include/
        //     types.h
        //   src/
        //     main.p4

        let include_dir = dir.path().join("include");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&include_dir).unwrap();
        fs::create_dir_all(&src_dir).unwrap();

        // Create header file
        let header = include_dir.join("types.h");
        fs::write(&header, "header my_header_t { bit<8> field; }").unwrap();

        // Create main P4 file that includes the header
        let main_p4 = src_dir.join("main.p4");
        fs::write(&main_p4, "#include \"types.h\"\nstruct metadata_t { my_header_t hdr; }")
            .unwrap();

        // Create config
        let config_path = dir.path().join(".p4lsp.json");
        let mut config_file = fs::File::create(&config_path).unwrap();
        writeln!(
            config_file,
            r#"{{
                "version": 1,
                "includePaths": ["include"],
                "targets": [
                    {{"root": "src/main.p4"}}
                ]
            }}"#
        )
        .unwrap();

        // Create project and set workspace root
        let mut project = ProjectState::new();
        project.set_workspace_root(dir.path().to_path_buf());

        // Verify config was loaded
        assert!(project.config.is_some());
        let config = project.config.as_ref().unwrap();
        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.include_paths.len(), 1);

        // Discover roots should return explicit target
        let roots = project.discover_roots();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with("main.p4"));

        // Compile root should work (include should be resolved via includePaths)
        let unit = project.compile_root(&roots[0]);
        assert!(unit.is_some(), "compile_root should succeed");

        let unit = unit.unwrap();
        // Should have 2 files: types.h and main.p4
        assert_eq!(unit.include_order.len(), 2);
        // Should have symbols from both files
        assert!(unit.symbols.symbols.len() >= 2); // at least my_header_t and metadata_t
    }

    #[test]
    fn test_auto_discovery_without_config() {
        let dir = TempDir::new().unwrap();

        // Create two P4 files, one includes the other
        let header = dir.path().join("common.p4");
        fs::write(&header, "struct common_t { bit<8> x; }").unwrap();

        let main = dir.path().join("main.p4");
        fs::write(&main, "#include \"common.p4\"\nstruct main_t { common_t c; }").unwrap();

        let mut project = ProjectState::new();
        project.set_workspace_root(dir.path().to_path_buf());

        // No config
        assert!(project.config.is_none());

        // Auto-discovery should find only main.p4 (common.p4 is included)
        let roots = project.discover_roots();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with("main.p4"));
    }
}
