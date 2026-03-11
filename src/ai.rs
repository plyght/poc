use crate::config::AiConfig;
use crate::types::*;
use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct AiFixer {
    config: AiConfig,
    max_fixes: Option<usize>,
}

impl AiFixer {
    pub fn new(config: AiConfig) -> Self {
        Self {
            config,
            max_fixes: None,
        }
    }

    pub fn with_max_fixes(mut self, max: Option<usize>) -> Self {
        self.max_fixes = max;
        self
    }

    pub fn with_overrides(mut self, provider: Option<&str>, model: Option<&str>) -> Self {
        if let Some(p) = provider {
            self.config.provider = p.to_string();
        }
        if let Some(m) = model {
            self.config.model = m.to_string();
        }
        self
    }

    pub fn fix_diagnostics(
        &self,
        diagnostics: &[(PathBuf, LintDiagnostic)],
        _plugins: &[Box<dyn Plugin>],
    ) -> Result<FixReport> {
        if diagnostics.is_empty() {
            println!("{}", "no errors to fix".green());
            return Ok(FixReport {
                fixed: 0,
                failed: 0,
                rolled_back: 0,
            });
        }

        println!(
            "ai-fix via {} ({}) — {} error{}",
            self.config.provider,
            self.config.model.dimmed(),
            diagnostics.len(),
            if diagnostics.len() == 1 { "" } else { "s" }
        );

        let mut grouped: HashMap<PathBuf, Vec<&LintDiagnostic>> = HashMap::new();
        for (project_path, diag) in diagnostics {
            let file_path = if Path::new(&diag.file).is_absolute() {
                PathBuf::from(&diag.file)
            } else {
                project_path.join(&diag.file)
            };
            grouped.entry(file_path).or_default().push(diag);
        }

        let file_groups: Vec<_> = grouped
            .into_iter()
            .take(self.max_fixes.unwrap_or(usize::MAX))
            .collect();

        let mut report = FixReport {
            fixed: 0,
            failed: 0,
            rolled_back: 0,
        };

        let total_start = Instant::now();

        for (file_path, diags) in &file_groups {
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(_) => {
                    report.failed += 1;
                    continue;
                }
            };

            let backup = source.clone();
            let fallback = file_path.display().to_string();
            let file_display = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&fallback)
                .to_string();

            println!(
                "  fixing {} ({} error{})",
                file_display,
                diags.len(),
                if diags.len() == 1 { "" } else { "s" }
            );

            let mut error_lines = String::new();
            for d in diags.iter() {
                error_lines.push_str(&format!(
                    "- Line {}:{} [{}]: {}\n",
                    d.line, d.col, d.rule, d.message
                ));
                if let Some(ref s) = d.suggestion {
                    error_lines.push_str(&format!("  {}\n", s));
                }
            }

            let prompt = format!(
                "Fix these errors in `{file_display}`:\n\n\
                 {error_lines}\n\
                 Code:\n```\n{source}\n```\n\n\
                 Return the complete fixed file. No explanations, just code."
            );

            println!(
                "    requesting fix from {}...",
                self.config.provider.dimmed()
            );

            let call_start = Instant::now();
            let llm_result = match self.config.provider.as_str() {
                "anthropic" => self.call_anthropic(&prompt),
                "ollama" => self.call_ollama(&prompt),
                _ => self.call_openai_compatible(&prompt),
            };
            let call_elapsed = call_start.elapsed();

            match llm_result {
                Ok(response) => {
                    let min_len = source.len() / 2;
                    if response.is_empty() || response.len() < min_len {
                        std::fs::write(file_path, &backup)?;
                        report.rolled_back += 1;
                        println!("    {} response too short, rolled back", "✗".red());
                    } else {
                        let fixed = strip_code_fences(&response);
                        std::fs::write(file_path, fixed)?;
                        report.fixed += 1;
                        println!(
                            "    {} applied {}",
                            "✓".green(),
                            format!("[{}ms]", call_elapsed.as_millis()).dimmed()
                        );
                    }
                }
                Err(e) => {
                    report.failed += 1;
                    println!("    {} {e}", "✗".red());
                }
            }
        }

        let total_elapsed = total_start.elapsed();
        println!();
        println!(
            "{} fixed, {} failed, {} rolled back {}",
            report.fixed,
            report.failed,
            report.rolled_back,
            format!("[{}ms]", total_elapsed.as_millis()).dimmed()
        );

        Ok(report)
    }

    fn call_ollama(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.config.endpoint);
        let body = json!({
            "model": self.config.model,
            "prompt": prompt,
            "stream": false,
        });

        let resp = reqwest::blocking::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .context("failed to reach ollama")?;

        let json: serde_json::Value = resp.json()?;
        json.get("response")
            .and_then(|r| r.as_str())
            .map(|s| s.trim().to_string())
            .context("no response from ollama")
    }

    fn call_anthropic(&self, prompt: &str) -> Result<String> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
        let body = json!({
            "model": self.config.model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}]
        });

        let resp = reqwest::blocking::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .context("failed to reach anthropic")?;

        let json: serde_json::Value = resp.json()?;
        json.get("content")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|block| block.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.trim().to_string())
            .context("no response from anthropic")
    }

    fn call_openai_compatible(&self, prompt: &str) -> Result<String> {
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        let url = format!("{}/v1/chat/completions", self.config.endpoint);
        let body = json!({
            "model": self.config.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 4096,
        });

        let resp = reqwest::blocking::Client::new()
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .context("failed to reach openai-compatible endpoint")?;

        let json: serde_json::Value = resp.json()?;
        json.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.trim().to_string())
            .context("no response from openai-compatible endpoint")
    }
}

pub struct FixReport {
    pub fixed: usize,
    pub failed: usize,
    pub rolled_back: usize,
}

fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("```") {
        let after_lang =
            inner.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
        let after_newline = after_lang.strip_prefix('\n').unwrap_or(after_lang);
        if let Some(body) = after_newline.strip_suffix("```") {
            return body.trim_end_matches('\n');
        }
    }
    s
}

#[allow(dead_code)]
fn count_project_errors(project_path: &Path, plugins: &[Box<dyn Plugin>]) -> usize {
    let lang = detect_language_from_path(project_path);
    let plugin = match plugins.iter().find(|p| p.language() == lang) {
        Some(p) => p,
        None => return 0,
    };

    let mut count = 0;

    if let Ok(build_result) = plugin.build(
        project_path,
        &BuildOpts {
            release: false,
            test: false,
            run: false,
            verbose: false,
            filter: None,
        },
    ) {
        count += build_result.errors.len();
        if !build_result.success {
            count = count.max(1);
        }
    }

    if let Ok(lint_result) = plugin.lint(project_path, &LintOpts { fix: false, verbose: false }) {
        count += lint_result.diagnostics.len();
    }

    count
}

#[allow(dead_code)]
fn extract_context(source: &str, line: usize, radius: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = line.saturating_sub(radius + 1);
    let end = (line + radius).min(lines.len());
    lines[start..end].join("\n")
}

#[allow(dead_code)]
fn apply_hunk(
    lines: &mut Vec<String>,
    start: usize,
    offset: &mut isize,
    removals: &[String],
    additions: &[String],
) {
    let actual_start = (start as isize + *offset) as usize;
    if actual_start + removals.len() > lines.len() {
        return;
    }

    for _ in 0..removals.len() {
        if actual_start < lines.len() {
            lines.remove(actual_start);
        }
    }

    for (i, line) in additions.iter().enumerate() {
        if actual_start + i <= lines.len() {
            lines.insert(actual_start + i, line.clone());
        }
    }

    *offset += additions.len() as isize - removals.len() as isize;
}

#[allow(dead_code)]
fn detect_language_from_path(path: &Path) -> Language {
    if path.join("Cargo.toml").exists() {
        return Language::Rust;
    }
    if path.join("go.mod").exists() {
        return Language::Go;
    }
    if path.join("package.json").exists() {
        return Language::TypeScript;
    }
    if path.join("CMakeLists.txt").exists() || path.join("Makefile").exists() {
        return Language::C;
    }
    if path.join("pyproject.toml").exists() {
        return Language::Python;
    }
    if path.join("build.zig").exists() {
        return Language::Zig;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => Language::Rust,
        "go" => Language::Go,
        "ts" | "tsx" | "js" | "jsx" => Language::TypeScript,
        "c" | "cpp" | "h" | "hpp" => Language::C,
        "py" => Language::Python,
        "zig" => Language::Zig,
        _ => Language::C,
    }
}
