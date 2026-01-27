use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use p4::ast::AST;
use p4::check::{self, Diagnostics};

use p4::lexer::Lexer;
use p4::parser::Parser;
use p4::preprocessor::MacroEnv;

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

#[derive(Debug)]
pub struct RootUnit {
    pub root: FilePath,
    pub include_order: Vec<FilePath>,
    pub include_index: HashMap<FilePath, usize>,
    pub symbols: SymbolTable,
    pub macro_env: MacroEnv,
    pub ast: AST,
    pub diagnostics: Diagnostics,
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
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            workspace_root: None,
            vfs: DashMap::new(),
            roots: DashMap::new(),
            file_to_roots: DashMap::new(),
            include_paths: Vec::new(),
        }
    }

    pub fn set_workspace_root(&mut self, root: PathBuf) {
        self.workspace_root = Some(root);
    }

    pub fn add_include_path(&mut self, path: PathBuf) {
        self.include_paths.push(path);
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
        let Some(ref workspace) = self.workspace_root else {
            return Vec::new();
        };

        let mut all_files: HashSet<FilePath> = HashSet::new();
        let mut included_files: HashSet<FilePath> = HashSet::new();

        Self::collect_p4_files(workspace, &mut all_files);

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

        all_files.difference(&included_files).cloned().collect()
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

    pub fn resolve_include(&self, from_file: &FilePath, include: &str) -> Option<FilePath> {
        let from_path = Path::new(from_file.as_str());
        let parent = from_path.parent().unwrap_or(Path::new("."));

        let relative = parent.join(include);
        if relative.exists() {
            return Some(Arc::new(
                relative.canonicalize().ok()?.to_string_lossy().to_string(),
            ));
        }

        for inc_path in &self.include_paths {
            let candidate = inc_path.join(include);
            if candidate.exists() {
                return Some(Arc::new(
                    candidate.canonicalize().ok()?.to_string_lossy().to_string(),
                ));
            }
        }

        None
    }

    pub fn compile_root(&self, root_path: &FilePath) -> Option<RootUnit> {
        let text = self.read_file(root_path)?;

        let mut ast = AST::default();
        let mut symbols = SymbolTable::new();
        let mut include_order = Vec::new();
        let mut include_index = HashMap::new();
        let mut visited = HashSet::new();

        let env = self.process_file_recursive(
            root_path,
            &text,
            &MacroEnv::default(),
            &mut ast,
            &mut symbols,
            &mut include_order,
            &mut include_index,
            &mut visited,
        )?;

        let (_, diagnostics) = check::all(&ast);

        Some(RootUnit {
            root: root_path.clone(),
            include_order,
            include_index,
            symbols,
            macro_env: env,
            ast,
            diagnostics,
        })
    }

    fn process_file_recursive(
        &self,
        file_path: &FilePath,
        text: &str,
        env_in: &MacroEnv,
        ast: &mut AST,
        symbols: &mut SymbolTable,
        include_order: &mut Vec<FilePath>,
        include_index: &mut HashMap<FilePath, usize>,
        visited: &mut HashSet<FilePath>,
    ) -> Option<MacroEnv> {
        if visited.contains(file_path) {
            return Some(env_in.clone());
        }
        visited.insert(file_path.clone());

        let ppr = p4::preprocessor::run_with_env(text, file_path.clone(), env_in).ok()?;
        let mut current_env = ppr.env_out.clone();

        for inc in &ppr.elements.includes {
            if let Some(resolved) = self.resolve_include(file_path, inc) {
                if let Some(inc_text) = self.read_file(&resolved) {
                    current_env = self.process_file_recursive(
                        &resolved,
                        &inc_text,
                        &current_env,
                        ast,
                        symbols,
                        include_order,
                        include_index,
                        visited,
                    )?;
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
        parser.run(ast).ok()?;

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
