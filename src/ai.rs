use crate::config::AiConfig;
use crate::types::*;
use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::json;
use std::path::Path;

pub struct AiFixer {
    config: AiConfig,
}

impl AiFixer {
    pub fn new(config: AiConfig) -> Self {
        Self { config }
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
        diagnostics: &[(std::path::PathBuf, LintDiagnostic)],
        plugins: &[Box<dyn Plugin>],
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
            "{} {} errors to fix via {} ({})",
            "ai-fix:".cyan().bold(),
            diagnostics.len(),
            self.config.provider,
            self.config.model
        );

        let mut report = FixReport {
            fixed: 0,
            failed: 0,
            rolled_back: 0,
        };

        for (project_path, diag) in diagnostics {
            let file_path = if Path::new(&diag.file).is_absolute() {
                std::path::PathBuf::from(&diag.file)
            } else {
                project_path.join(&diag.file)
            };

            let source = match std::fs::read_to_string(&file_path) {
                Ok(s) => s,
                Err(_) => {
                    report.failed += 1;
                    continue;
                }
            };

            let context = extract_context(&source, diag.line, 10);
            let backup = source.clone();

            println!(
                "  {} {}:{}:{} — {}",
                "fixing".yellow(),
                diag.file,
                diag.line,
                diag.col,
                diag.message.dimmed()
            );

            let error_count_before = count_project_errors(project_path, plugins);

            match self.get_fix(&diag.file, &context, diag) {
                Ok(diff_output) => {
                    let new_source = apply_diff(&source, &diff_output, diag.line, 10);
                    std::fs::write(&file_path, &new_source)?;

                    let error_count_after = count_project_errors(project_path, plugins);

                    if error_count_after >= error_count_before {
                        std::fs::write(&file_path, backup)?;
                        report.rolled_back += 1;
                        println!(
                            "    {} error count did not decrease ({} -> {}), rolled back",
                            "✗".red(),
                            error_count_before,
                            error_count_after
                        );
                    } else {
                        report.fixed += 1;
                        println!(
                            "    {} applied ({} -> {} errors)",
                            "✓".green(),
                            error_count_before,
                            error_count_after
                        );
                    }
                }
                Err(e) => {
                    report.failed += 1;
                    println!("    {} {e}", "✗".red());
                }
            }
        }

        println!(
            "\n{} {} fixed, {} failed, {} rolled back",
            "ai-fix summary:".bold(),
            report.fixed,
            report.failed,
            report.rolled_back
        );

        Ok(report)
    }

    fn get_fix(&self, file: &str, context: &str, diag: &LintDiagnostic) -> Result<String> {
        let prompt = format!(
            "Fix this error in `{file}`.\n\n\
             Error: {} [{}] at line {}:{}\n\
             {}\n\n\
             Code context:\n```\n{context}\n```\n\n\
             Return a unified diff (--- a/file, +++ b/file, @@ hunks) for the fix. \
             If you cannot produce a valid diff, return ONLY the fixed code for the context shown. \
             No explanations.",
            diag.message,
            diag.rule,
            diag.line,
            diag.col,
            diag.suggestion.as_deref().unwrap_or("")
        );

        match self.config.provider.as_str() {
            "anthropic" => self.call_anthropic(&prompt),
            "ollama" => self.call_ollama(&prompt),
            _ => self.call_openai_compatible(&prompt),
        }
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
        },
    ) {
        count += build_result.errors.len();
        if !build_result.success {
            count = count.max(1);
        }
    }

    if let Ok(lint_result) = plugin.lint(project_path, &LintOpts { fix: false }) {
        count += lint_result.diagnostics.len();
    }

    count
}

fn extract_context(source: &str, line: usize, radius: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = line.saturating_sub(radius + 1);
    let end = (line + radius).min(lines.len());
    lines[start..end].join("\n")
}

fn apply_diff(source: &str, ai_output: &str, line: usize, radius: usize) -> String {
    if ai_output.contains("---") && ai_output.contains("+++") && ai_output.contains("@@") {
        if let Some(result) = try_apply_unified_diff(source, ai_output) {
            return result;
        }
    }

    apply_context_fix(source, line, radius, ai_output)
}

fn try_apply_unified_diff(source: &str, diff: &str) -> Option<String> {
    let mut lines: Vec<String> = source.lines().map(|l| l.to_string()).collect();
    let mut in_hunk = false;
    let mut hunk_start: usize = 0;
    let mut offset: isize = 0;
    let mut removals = Vec::new();
    let mut additions = Vec::new();

    let re = regex::Regex::new(r"@@ -(\d+)").ok()?;
    for diff_line in diff.lines() {
        if diff_line.starts_with("@@") {
            if in_hunk && !removals.is_empty() {
                apply_hunk(&mut lines, hunk_start, &mut offset, &removals, &additions);
                removals.clear();
                additions.clear();
            }
            in_hunk = true;
            let cap = re.captures(diff_line)?;
            hunk_start = cap[1].parse::<usize>().ok()?.saturating_sub(1);
        } else if in_hunk {
            if let Some(stripped) = diff_line.strip_prefix('-') {
                removals.push(stripped.to_string());
            } else if let Some(stripped) = diff_line.strip_prefix('+') {
                additions.push(stripped.to_string());
            } else if diff_line.starts_with("---") || diff_line.starts_with("+++") {
                continue;
            }
        }
    }

    if in_hunk && !removals.is_empty() {
        apply_hunk(&mut lines, hunk_start, &mut offset, &removals, &additions);
    }

    if !removals.is_empty() || in_hunk {
        Some(lines.join("\n"))
    } else {
        None
    }
}

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

fn apply_context_fix(source: &str, line: usize, radius: usize, fixed: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = line.saturating_sub(radius + 1);
    let end = (line + radius).min(lines.len());

    let mut result = Vec::new();
    result.extend_from_slice(&lines[..start]);
    for fixed_line in fixed.lines() {
        result.push(fixed_line);
    }
    if end < lines.len() {
        result.extend_from_slice(&lines[end..]);
    }
    result.join("\n")
}

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
