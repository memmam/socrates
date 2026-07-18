//! The module loader: resolves `import` statements to files, parses each
//! module once, and returns all modules in dependency order (root last).
//!
//! `import a.b;` in `/dir/file.soc` loads `/dir/a/b.soc`. Every module is
//! identified by a *key* — its import path as first encountered ("a.b") — used
//! as the name-mangling prefix inside the shared checker; the root module's
//! key is empty (its names stay unprefixed). Files are deduplicated by
//! canonical path, so a diamond `main → a → c, main → b → c` loads `c` once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{Program, StmtKind};
use crate::diag::Diagnostic;
use crate::source::Source;
use crate::span::Span;
use crate::{diag, lexer, parser};

/// What `ModuleSession::load_imports` returns: the newly loaded units in
/// dependency order, the chunk's alias → key map, and the next free NodeId.
pub type LoadedImports = (Vec<ModuleUnit>, HashMap<String, String>, u32);

#[derive(Clone)]
pub struct ModuleUnit {
    /// Name-mangling prefix: "" for the root module, the module key otherwise.
    pub prefix: String,
    pub source: Source,
    pub program: Program,
    /// Import alias (as written or defaulted) → module key.
    pub imports: HashMap<String, String>,
}

/// Load `root` and everything it transitively imports. Modules come back in
/// dependency order (imports before importers, root last), each parsed with
/// globally unique `NodeId`s so they can share one checker.
///
/// Imports resolve relative to the importing file first, then against each
/// directory in the `SOCRATES_PATH` environment variable (colon-separated) —
/// the home for utility modules shared across projects.
///
/// On failure, returns the diagnostics together with the source they belong
/// to (lex/parse errors of any module, unreadable files, cycles).
pub fn load_modules(root: &Path) -> Result<Vec<ModuleUnit>, (Source, Vec<Diagnostic>)> {
    let search: Vec<PathBuf> = std::env::var("SOCRATES_PATH")
        .ok()
        .map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default();
    load_modules_with(root, &search)
}

/// `load_modules` with an explicit search path (testable without touching
/// the process environment).
pub fn load_modules_with(
    root: &Path,
    search: &[PathBuf],
) -> Result<Vec<ModuleUnit>, (Source, Vec<Diagnostic>)> {
    load_modules_overlay(root, search, &HashMap::new())
}

/// `load_modules_with`, reading some files from an in-memory overlay instead
/// of disk (keyed by canonical path). The language server uses this to check
/// unsaved editor buffers.
pub fn load_modules_overlay(
    root: &Path,
    search: &[PathBuf],
    overlay: &HashMap<PathBuf, String>,
) -> Result<Vec<ModuleUnit>, (Source, Vec<Diagnostic>)> {
    let mut loader = Loader {
        units: Vec::new(),
        key_by_path: HashMap::new(),
        keys_taken: HashMap::new(),
        loading: Vec::new(),
        next_id: 0,
        search: search.to_vec(),
        overlay,
    };
    loader.load(root, String::new())?;
    Ok(loader.units)
}

/// Persistent module-loading state for an interactive session: modules
/// already loaded in earlier chunks are never loaded twice, and NodeId
/// space is threaded across loads.
#[derive(Default, Clone)]
pub struct ModuleSession {
    key_by_path: HashMap<PathBuf, String>,
    keys_taken: HashMap<String, u32>,
    search: Vec<PathBuf>,
}

impl ModuleSession {
    pub fn new() -> ModuleSession {
        let search: Vec<PathBuf> = std::env::var("SOCRATES_PATH")
            .ok()
            .map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        ModuleSession { key_by_path: HashMap::new(), keys_taken: HashMap::new(), search }
    }

    /// Load the import targets of an interactive chunk. Paths resolve against
    /// `base_dir` (the session's working directory), then the search path;
    /// `std.*` resolves embedded. Returns the newly loaded units (dependency
    /// order), the chunk's alias map, and the next free NodeId.
    pub fn load_imports(
        &mut self,
        program: &Program,
        base_dir: &Path,
        importer: &Source,
        next_id: u32,
        overlay: &HashMap<PathBuf, String>,
    ) -> Result<LoadedImports, (Source, Vec<Diagnostic>)> {
        let mut loader = Loader {
            units: Vec::new(),
            key_by_path: std::mem::take(&mut self.key_by_path),
            keys_taken: std::mem::take(&mut self.keys_taken),
            loading: Vec::new(),
            next_id,
            search: self.search.clone(),
            overlay,
        };
        let result = loader.scan_imports(program, base_dir, importer, false);
        self.key_by_path = loader.key_by_path;
        self.keys_taken = loader.keys_taken;
        match result {
            Ok(imports) => Ok((loader.units, imports, loader.next_id)),
            Err(e) => Err(e),
        }
    }
}

struct Loader<'a> {
    units: Vec<ModuleUnit>,
    /// Canonical path → module key, for diamond dedup.
    key_by_path: HashMap<PathBuf, String>,
    /// Keys already claimed (uniqueness backstop for same-named modules from
    /// different directories).
    keys_taken: HashMap<String, u32>,
    /// DFS stack of (canonical path, display name) for cycle reporting.
    loading: Vec<(PathBuf, String)>,
    next_id: u32,
    /// Extra directories imports resolve against, after the importing file's
    /// own directory (`SOCRATES_PATH`).
    search: Vec<PathBuf>,
    /// In-memory file contents that shadow the disk (canonical path → text).
    overlay: &'a HashMap<PathBuf, String>,
}

impl Loader<'_> {
    fn load(
        &mut self,
        path: &Path,
        key: String,
    ) -> Result<(), (Source, Vec<Diagnostic>)> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let display = path.display().to_string();
        let text = if let Some(t) = self.overlay.get(&canon) {
            t.clone()
        } else {
            match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    let source = Source::new(display.clone(), String::new());
                    return Err((
                        source,
                        vec![Diagnostic::error(
                            "E0335",
                            format!("cannot read `{display}`: {e}"),
                        )],
                    ));
                }
            }
        };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        self.load_source(canon, display, text, key, dir, false)
    }

    /// Load an embedded standard-library module (`import std.x;`). The
    /// pseudo-path `<std.x>` keys the dedup and cycle maps.
    fn load_std(&mut self, key: &str) -> Result<(), (Source, Vec<Diagnostic>)> {
        let text = crate::stdlib::std_module(key)
            .expect("caller checked the std module exists")
            .to_string();
        let canon = PathBuf::from(format!("<{key}>"));
        self.load_source(canon, format!("<{key}>"), text, key.to_string(), PathBuf::new(), true)
    }

    fn load_source(
        &mut self,
        canon: PathBuf,
        display: String,
        text: String,
        key: String,
        dir: PathBuf,
        is_std: bool,
    ) -> Result<(), (Source, Vec<Diagnostic>)> {
        let source = Source::new(display.clone(), text.clone());

        let lexed = lexer::lex(&text);
        let parsed = parser::parse_with_ids(lexed.tokens, &text, self.next_id);
        let mut diags = lexed.diags;
        diags.extend(parsed.diags);
        if diag::has_errors(&diags) {
            return Err((source, diags));
        }
        self.next_id = parsed.node_count;

        self.key_by_path.insert(canon.clone(), key.clone());
        self.loading.push((canon, display));

        let imports = self.scan_imports(&parsed.program, &dir, &source, is_std)?;

        self.loading.pop();
        self.units.push(ModuleUnit {
            prefix: key,
            source,
            program: parsed.program,
            imports,
        });
        Ok(())
    }

    /// Resolve and load every import statement in `program`, returning the
    /// alias → module key map for the importing scope.
    fn scan_imports(
        &mut self,
        program: &Program,
        dir: &Path,
        source: &Source,
        is_std: bool,
    ) -> Result<HashMap<String, String>, (Source, Vec<Diagnostic>)> {
        let mut imports: HashMap<String, String> = HashMap::new();
        for stmt in &program.stmts {
            let StmtKind::Import { path: segs, alias } = &stmt.kind else { continue };
            if is_std && segs[0].name != "std" {
                return Err((
                    source.clone(),
                    vec![Diagnostic::error(
                        "E0337",
                        "standard-library modules may only import other `std` modules",
                    )
                    .with_label(stmt.span, "")],
                ));
            }
            let target_key = self.resolve_import(dir, segs, stmt.span, source)?;
            let alias_name = alias
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| segs.last().unwrap().name.clone());
            if imports.insert(alias_name.clone(), target_key).is_some() {
                return Err((
                    source.clone(),
                    vec![Diagnostic::error(
                        "E0336",
                        format!("duplicate import alias `{alias_name}`"),
                    )
                    .with_label(stmt.span, "already imported under this name")
                    .with_note("use `as` to give one of them a different alias")],
                ));
            }
        }
        Ok(imports)
    }

    /// Resolve (and if necessary load) one import; returns the module key.
    fn resolve_import(
        &mut self,
        dir: &Path,
        segs: &[crate::ast::Ident],
        span: Span,
        importer: &Source,
    ) -> Result<String, (Source, Vec<Diagnostic>)> {
        let dotted: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
        let dotted = dotted.join(".");

        // `std.*` is reserved for the embedded standard library.
        if segs[0].name == "std" {
            let pseudo = PathBuf::from(format!("<{dotted}>"));
            if self.loading.iter().any(|(p, _)| *p == pseudo) {
                return Err((
                    importer.clone(),
                    vec![Diagnostic::error(
                        "E0338",
                        format!("circular import of `{dotted}`"),
                    )
                    .with_label(span, "this import completes a cycle")],
                ));
            }
            if let Some(k) = self.key_by_path.get(&pseudo) {
                return Ok(k.clone());
            }
            if crate::stdlib::std_module(&dotted).is_none() {
                return Err((
                    importer.clone(),
                    vec![Diagnostic::error(
                        "E0337",
                        format!("no standard-library module `{dotted}`"),
                    )
                    .with_label(span, "not part of the embedded std")
                    .with_note(format!(
                        "available: {}",
                        crate::stdlib::std_module_names().join(", ")
                    ))],
                ));
            }
            self.load_std(&dotted)?;
            return Ok(dotted);
        }

        let mut rel = PathBuf::new();
        for s in segs {
            rel.push(&s.name);
        }
        rel.set_extension("soc");
        // File-relative first, then each SOCRATES_PATH directory.
        let mut tried = Vec::new();
        let mut found = None;
        for base in std::iter::once(dir).chain(self.search.iter().map(PathBuf::as_path)) {
            let candidate = base.join(&rel);
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
            tried.push(candidate);
        }
        let Some(target) = found else {
            let mut d = Diagnostic::error("E0337", format!("cannot find module `{dotted}`"))
                .with_label(span, format!("looked for `{}`", tried[0].display()));
            for t in &tried[1..] {
                d = d.with_note(format!("also tried `{}` (SOCRATES_PATH)", t.display()));
            }
            return Err((importer.clone(), vec![d]));
        };
        let canon = target.canonicalize().unwrap_or_else(|_| target.clone());

        if self.loading.iter().any(|(p, _)| *p == canon) {
            let chain: Vec<&str> = self
                .loading
                .iter()
                .map(|(_, name)| name.as_str())
                .collect();
            return Err((
                importer.clone(),
                vec![Diagnostic::error(
                    "E0338",
                    format!("circular import of `{dotted}`"),
                )
                .with_label(span, "this import completes a cycle")
                .with_note(format!("import chain: {}", chain.join(" → ")))],
            ));
        }
        if let Some(k) = self.key_by_path.get(&canon) {
            return Ok(k.clone());
        }

        // Claim a unique key: the dotted import path, disambiguated if a
        // different file already took it.
        let n = self.keys_taken.entry(dotted.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { dotted } else { format!("{dotted}#{n}") };
        self.load(&target, key.clone())?;
        Ok(key)
    }
}
