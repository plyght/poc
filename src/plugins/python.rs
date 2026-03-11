use crate::config::PocConfig;
use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct PythonPlugin {
    runner: String,
    linter: String,
}

impl PythonPlugin {
    pub fn new(config: &PocConfig) -> Self {
        Self {
            runner: config.python.runner.clone(),
            linter: config.lint.python.clone(),
        }
    }

    fn has_pyproject(path: &Path) -> bool {
        path.join("pyproject.toml").exists()
    }
}

impl Plugin for PythonPlugin {
    fn language(&self) -> Language {
        Language::Python
    }

    fn detect(&self, path: &Path) -> bool {
        Self::has_pyproject(path)
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let build_dir = path.join("build");
        std::fs::create_dir_all(&build_dir)?;

        let output = match self.runner.as_str() {
            "uv" => Command::new("uv")
                .args(["sync"])
                .current_dir(path)
                .output()?,
            "poetry" => Command::new("poetry")
                .args(["install"])
                .current_dir(path)
                .output()?,
            "pdm" => Command::new("pdm")
                .args(["install"])
                .current_dir(path)
                .output()?,
            _ => Command::new("pip")
                .args(["install", "-e", "."])
                .current_dir(path)
                .output()?,
        };

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let _ = opts.release;

        if opts.test && output.status.success() {
            let test_out = match self.runner.as_str() {
                "uv" => Command::new("uv")
                    .args(["run", "pytest"])
                    .current_dir(path)
                    .output()?,
                "poetry" => Command::new("poetry")
                    .args(["run", "pytest"])
                    .current_dir(path)
                    .output()?,
                _ => Command::new("pytest").current_dir(path).output()?,
            };
            let test_stderr = String::from_utf8_lossy(&test_out.stderr).to_string();
            let test_stdout = String::from_utf8_lossy(&test_out.stdout).to_string();
            return Ok(BuildResult {
                success: test_out.status.success(),
                output: format!("{test_stdout}\n{test_stderr}"),
                errors: vec![],
            });
        }

        if opts.run && output.status.success() {
            let run_out = match self.runner.as_str() {
                "uv" => Command::new("uv")
                    .args(["run", "python", "-m", guess_module_name(path)])
                    .current_dir(path)
                    .output()?,
                "poetry" => Command::new("poetry")
                    .args(["run", "python", "-m", guess_module_name(path)])
                    .current_dir(path)
                    .output()?,
                _ => Command::new("python")
                    .args(["-m", guess_module_name(path)])
                    .current_dir(path)
                    .output()?,
            };
            let run_stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
            let run_stderr = String::from_utf8_lossy(&run_out.stderr).to_string();
            return Ok(BuildResult {
                success: run_out.status.success(),
                output: format!("{run_stdout}\n{run_stderr}"),
                errors: vec![],
            });
        }

        Ok(BuildResult {
            success: output.status.success(),
            output: stderr,
            errors: vec![],
        })
    }

    fn lint(&self, path: &Path, opts: &LintOpts) -> Result<LintResult> {
        let output = match self.linter.as_str() {
            "ruff" => {
                let mut args = vec!["check", "--output-format=json"];
                if opts.fix {
                    args.push("--fix");
                }
                args.push(".");
                match self.runner.as_str() {
                    "uv" => {
                        let mut full = vec!["run", "ruff"];
                        full.extend_from_slice(&args);
                        Command::new("uv").args(&full).current_dir(path).output()?
                    }
                    _ => Command::new("ruff")
                        .args(&args)
                        .current_dir(path)
                        .output()?,
                }
            }
            "pylint" => {
                let mut args = vec!["--output-format=json"];
                if let Some(src) = find_python_src(path) {
                    args.push(src.to_str().unwrap_or("."));
                } else {
                    args.push(".");
                }
                let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                Command::new("pylint")
                    .args(&args_owned)
                    .current_dir(path)
                    .output()?
            }
            _ => {
                let mut args = vec!["--format=json"];
                args.push(".");
                Command::new("flake8")
                    .args(&args)
                    .current_dir(path)
                    .output()?
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let diagnostics = if self.linter == "ruff" {
            parse_ruff_json(&stdout)
        } else {
            parse_python_lint_output(&stdout)
        };

        Ok(LintResult {
            success: output.status.success()
                && diagnostics.iter().all(|d| d.severity != Severity::Error),
            diagnostics,
        })
    }

    fn clean(&self, path: &Path) -> Result<()> {
        let build_dir = path.join("build");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        for dir in &["__pycache__", ".pytest_cache", "*.egg-info", ".ruff_cache"] {
            let _ = Command::new("find")
                .args([
                    path.to_str().unwrap_or("."),
                    "-name",
                    dir,
                    "-exec",
                    "rm",
                    "-rf",
                    "{}",
                    "+",
                ])
                .output();
        }
        Ok(())
    }
}

fn find_python_src(path: &Path) -> Option<&Path> {
    if path.join("src").exists() {
        Some(path)
    } else {
        None
    }
}

fn guess_module_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("__main__")
}

fn parse_ruff_json(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(output) {
        for item in items {
            let file = item
                .get("filename")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            let line = item
                .get("location")
                .and_then(|l| l.get("row"))
                .and_then(|r| r.as_u64())
                .unwrap_or(0) as usize;
            let col = item
                .get("location")
                .and_then(|l| l.get("column"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0) as usize;
            let code = item
                .get("code")
                .and_then(|c| c.as_str())
                .unwrap_or("unknown")
                .to_string();
            let message = item
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let fix = item
                .get("fix")
                .and_then(|f| f.get("message"))
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());

            diags.push(LintDiagnostic {
                file,
                line,
                col,
                rule: code,
                severity: Severity::Warning,
                message,
                suggestion: fix,
            });
        }
    }
    diags
}

fn parse_python_lint_output(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(.+\.py):(\d+):(\d+):\s*(\w+)\s+(.+)").unwrap();
    for cap in re.captures_iter(output) {
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: cap[4].to_string(),
            severity: Severity::Warning,
            message: cap[5].to_string(),
            suggestion: None,
        });
    }
    diags
}
