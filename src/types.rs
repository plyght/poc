use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintDiagnostic {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct BuildResult {
    pub success: bool,
    pub output: String,
    pub errors: Vec<LintDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct LintResult {
    pub success: bool,
    pub diagnostics: Vec<LintDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Go,
    TypeScript,
    C,
    Python,
    Zig,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Language::Rust => write!(f, "rust"),
            Language::Go => write!(f, "go"),
            Language::TypeScript => write!(f, "typescript"),
            Language::C => write!(f, "c"),
            Language::Python => write!(f, "python"),
            Language::Zig => write!(f, "zig"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectedProject {
    pub path: std::path::PathBuf,
    pub language: Language,
}

pub struct BuildOpts {
    pub release: bool,
    pub test: bool,
    pub run: bool,
}

pub struct LintOpts {
    pub fix: bool,
}

pub trait Plugin: Send + Sync {
    fn language(&self) -> Language;
    fn detect(&self, path: &Path) -> bool;
    fn build(&self, path: &Path, opts: &BuildOpts) -> anyhow::Result<BuildResult>;
    fn lint(&self, path: &Path, opts: &LintOpts) -> anyhow::Result<LintResult>;
    fn clean(&self, path: &Path) -> anyhow::Result<()>;
}
