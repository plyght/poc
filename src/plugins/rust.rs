use crate::config::PocConfig;
use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct RustPlugin {
    linker: String,
    linter: String,
}

impl RustPlugin {
    pub fn new(config: &PocConfig) -> Self {
        Self {
            linker: config.rust.linker.clone(),
            linter: config.lint.rust.clone(),
        }
    }

    fn cargo(&self, path: &Path, args: &[&str]) -> Result<std::process::Output> {
        let mut cmd = Command::new("cargo");
        cmd.args(args).current_dir(path);

        if self.linker != "default" {
            cmd.env("RUSTFLAGS", format!("-C linker={}", self.linker));
        }

        Ok(cmd.output()?)
    }
}

impl Plugin for RustPlugin {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("Cargo.toml").exists()
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let mut args = vec!["build"];
        if opts.release {
            args.push("--release");
        }

        let output = self.cargo(path, &args)?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let errors = parse_cargo_diagnostics(&stderr);

        if output.status.success() {
            copy_artifacts(path, opts.release)?;
        }

        if opts.test && output.status.success() {
            let test_out = self.cargo(path, &["test"])?;
            let test_stderr = String::from_utf8_lossy(&test_out.stderr).to_string();
            return Ok(BuildResult {
                success: test_out.status.success(),
                output: test_stderr,
                errors,
            });
        }

        if opts.run && output.status.success() {
            let run_out = self.cargo(path, &["run"])?;
            let stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
            return Ok(BuildResult {
                success: run_out.status.success(),
                output: stdout,
                errors,
            });
        }

        Ok(BuildResult {
            success: output.status.success(),
            output: stderr,
            errors,
        })
    }

    fn lint(&self, path: &Path, opts: &LintOpts) -> Result<LintResult> {
        let args = if self.linter == "clippy" {
            let mut a = vec!["clippy", "--message-format=json"];
            if opts.fix {
                a = vec!["clippy", "--fix", "--allow-dirty", "--message-format=json"];
            }
            a
        } else {
            vec!["check", "--message-format=json"]
        };

        let output = self.cargo(path, &args)?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let diagnostics = parse_cargo_json_diagnostics(&stderr);

        Ok(LintResult {
            success: output.status.success(),
            diagnostics,
        })
    }

    fn clean(&self, path: &Path) -> Result<()> {
        self.cargo(path, &["clean"])?;
        Ok(())
    }
}

fn copy_artifacts(path: &Path, release: bool) -> Result<()> {
    let profile = if release { "release" } else { "debug" };
    let target_dir = path.join("target").join(profile);
    let build_dir = path.join("build");
    std::fs::create_dir_all(&build_dir)?;

    if target_dir.exists() {
        for entry in std::fs::read_dir(&target_dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            if !ft.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".d")
                || name_str.ends_with(".fingerprint")
                || name_str.starts_with("lib") && name_str.ends_with(".rlib")
                || name_str.contains(".cargo-lock")
            {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = entry.metadata() {
                    let mode = meta.permissions().mode();
                    if mode & 0o111 == 0
                        && !name_str.ends_with(".so")
                        && !name_str.ends_with(".dylib")
                        && !name_str.ends_with(".a")
                    {
                        continue;
                    }
                }
            }
            let dest = build_dir.join(&name);
            let _ = std::fs::copy(entry.path(), dest);
        }
    }
    Ok(())
}

fn parse_cargo_diagnostics(stderr: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(?m)^error\[?\w*\]?: (.+)\n\s*--> (.+):(\d+):(\d+)").unwrap();
    for cap in re.captures_iter(stderr) {
        diags.push(LintDiagnostic {
            file: cap[2].to_string(),
            line: cap[3].parse().unwrap_or(0),
            col: cap[4].parse().unwrap_or(0),
            rule: "compiler".into(),
            severity: Severity::Error,
            message: cap[1].to_string(),
            suggestion: None,
        });
    }
    diags
}

fn parse_cargo_json_diagnostics(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    for line in output.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if val.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                if let Some(msg) = val.get("message") {
                    let message = msg
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("error");
                    let severity = match level {
                        "warning" => Severity::Warning,
                        "error" => Severity::Error,
                        "note" | "help" => Severity::Info,
                        _ => Severity::Warning,
                    };

                    let (file, line_num, col) = msg
                        .get("spans")
                        .and_then(|s| s.as_array())
                        .and_then(|spans| spans.first())
                        .map(|span| {
                            (
                                span.get("file_name")
                                    .and_then(|f| f.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                span.get("line_start").and_then(|l| l.as_u64()).unwrap_or(0)
                                    as usize,
                                span.get("column_start")
                                    .and_then(|c| c.as_u64())
                                    .unwrap_or(0) as usize,
                            )
                        })
                        .unwrap_or_default();

                    let rule = msg
                        .get("code")
                        .and_then(|c| c.get("code"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let suggestion = msg
                        .get("children")
                        .and_then(|c| c.as_array())
                        .and_then(|children| children.first())
                        .and_then(|child| child.get("message"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());

                    if !file.is_empty() {
                        diags.push(LintDiagnostic {
                            file,
                            line: line_num,
                            col,
                            rule,
                            severity,
                            message,
                            suggestion,
                        });
                    }
                }
            }
        }
    }
    diags
}
