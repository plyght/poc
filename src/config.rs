use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Clone)]
pub struct PocConfig {
    #[serde(default)]
    pub ts: TsConfig,
    #[serde(default)]
    pub python: PythonConfig,
    #[serde(default)]
    pub c: CConfig,
    #[serde(default)]
    pub rust: RustConfig,
    #[serde(default)]
    pub lint: LintConfig,
    #[serde(default)]
    pub ai: AiConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TsConfig {
    #[serde(default = "default_bun")]
    pub runtime: String,
    #[serde(default = "default_bun")]
    pub package_manager: String,
}

impl Default for TsConfig {
    fn default() -> Self {
        Self {
            runtime: "bun".into(),
            package_manager: "bun".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct PythonConfig {
    #[serde(default = "default_uv")]
    pub runner: String,
}

impl Default for PythonConfig {
    fn default() -> Self {
        Self {
            runner: "uv".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CConfig {
    #[serde(default = "default_clang")]
    pub compiler: String,
    #[serde(default = "default_cmake")]
    pub build_system: String,
}

impl Default for CConfig {
    fn default() -> Self {
        Self {
            compiler: "clang".into(),
            build_system: "cmake".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RustConfig {
    #[serde(default = "default_default")]
    pub linker: String,
}

impl Default for RustConfig {
    fn default() -> Self {
        Self {
            linker: "default".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LintConfig {
    #[serde(default = "default_biome")]
    pub ts: String,
    #[serde(default = "default_ruff")]
    pub python: String,
    #[serde(default = "default_clippy")]
    pub rust: String,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            ts: "biome".into(),
            python: "ruff".into(),
            rust: "clippy".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AiConfig {
    #[serde(default = "default_ollama")]
    pub provider: String,
    #[serde(default = "default_llama3")]
    pub model: String,
    #[serde(default = "default_ollama_endpoint")]
    pub endpoint: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
            model: "llama3".into(),
            endpoint: "http://0.0.0.0:11434".into(),
        }
    }
}

fn default_bun() -> String {
    "bun".into()
}
fn default_uv() -> String {
    "uv".into()
}
fn default_clang() -> String {
    "clang".into()
}
fn default_cmake() -> String {
    "cmake".into()
}
fn default_default() -> String {
    "default".into()
}
fn default_biome() -> String {
    "biome".into()
}
fn default_ruff() -> String {
    "ruff".into()
}
fn default_clippy() -> String {
    "clippy".into()
}
fn default_ollama() -> String {
    "ollama".into()
}
fn default_llama3() -> String {
    "llama3".into()
}
fn default_ollama_endpoint() -> String {
    "http://0.0.0.0:11434".into()
}

impl PocConfig {
    pub fn load(project_root: &Path) -> Result<Self> {
        let local = project_root.join(".poc").join("config.toml");
        if local.exists() {
            let content = std::fs::read_to_string(&local)?;
            return Ok(toml::from_str(&content)?);
        }

        if let Some(global) = global_config_path() {
            if global.exists() {
                let content = std::fs::read_to_string(&global)?;
                return Ok(toml::from_str(&content)?);
            }
        }

        Ok(PocConfig::default())
    }
}

fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("poc").join("config.toml"))
}

pub fn generate_config(root: &Path, projects: &[crate::types::DetectedProject]) -> Result<()> {
    use crate::types::Language;
    let poc_dir = root.join(".poc");
    let _ = std::fs::create_dir_all(&poc_dir);
    let config_path = poc_dir.join("config.toml");
    if config_path.exists() {
        anyhow::bail!(
            ".poc/config.toml already exists at {}",
            config_path.display()
        );
    }

    let mut sections = Vec::new();
    let has_ts = projects.iter().any(|p| p.language == Language::TypeScript);
    let has_python = projects.iter().any(|p| p.language == Language::Python);
    let has_c = projects.iter().any(|p| p.language == Language::C);
    let has_rust = projects.iter().any(|p| p.language == Language::Rust);

    if has_ts {
        sections.push("[ts]\nruntime = \"bun\"\npackage_manager = \"bun\"".to_string());
    }
    if has_python {
        sections.push("[python]\nrunner = \"uv\"".to_string());
    }
    if has_c {
        sections.push("[c]\ncompiler = \"clang\"\nbuild_system = \"cmake\"".to_string());
    }
    if has_rust {
        sections.push("[rust]\nlinker = \"default\"".to_string());
    }

    let mut lint_entries = Vec::new();
    if has_ts {
        lint_entries.push("ts = \"biome\"".to_string());
    }
    if has_python {
        lint_entries.push("python = \"ruff\"".to_string());
    }
    if has_rust {
        lint_entries.push("rust = \"clippy\"".to_string());
    }
    if !lint_entries.is_empty() {
        sections.push(format!("[lint]\n{}", lint_entries.join("\n")));
    }

    sections.push(
        "[ai]\nprovider = \"ollama\"\nmodel = \"llama3\"\nendpoint = \"http://0.0.0.0:11434\""
            .to_string(),
    );

    let content = sections.join("\n\n");
    std::fs::write(&config_path, &content)?;
    Ok(())
}

pub fn validate_config(root: &Path) -> Vec<String> {
    let config_path = root.join(".poc").join("config.toml");
    let mut warnings = Vec::new();

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return warnings,
    };

    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("failed to parse .poc/config.toml: {e}"));
            return warnings;
        }
    };

    let known_sections = ["ts", "python", "c", "rust", "lint", "ai"];
    for key in table.keys() {
        if !known_sections.contains(&key.as_str()) {
            warnings.push(format!("unknown section [{key}] in .poc/config.toml"));
        }
    }

    if let Some(ai) = table.get("ai").and_then(|v| v.as_table()) {
        if let Some(provider) = ai.get("provider").and_then(|v| v.as_str()) {
            let valid = ["ollama", "anthropic", "openai"];
            if !valid.contains(&provider) {
                warnings.push(format!(
                    "unknown AI provider '{provider}' — expected one of: {}",
                    valid.join(", ")
                ));
            }
        }
    }

    warnings
}
