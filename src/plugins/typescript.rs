use crate::config::PocConfig;
use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct TypeScriptPlugin {
    runtime: String,
    package_manager: String,
    linter: String,
}

impl TypeScriptPlugin {
    pub fn new(config: &PocConfig) -> Self {
        Self {
            runtime: config.ts.runtime.clone(),
            package_manager: config.ts.package_manager.clone(),
            linter: config.lint.ts.clone(),
        }
    }
}

impl Plugin for TypeScriptPlugin {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("package.json").exists()
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let build_dir = path.join("build");
        std::fs::create_dir_all(&build_dir)?;

        self.install_deps(path)?;

        let output = match self.runtime.as_str() {
            "bun" => {
                if path.join("tsconfig.json").exists() {
                    Command::new("bun")
                        .args(["build", "./src/index.ts", "--outdir", "build"])
                        .current_dir(path)
                        .output()?
                } else {
                    return Ok(BuildResult {
                        success: true,
                        output: "no build step needed".into(),
                        errors: vec![],
                    });
                }
            }
            "deno" => Command::new("deno")
                .args(["compile", "--output", "build/main", "src/index.ts"])
                .current_dir(path)
                .output()?,
            _ => Command::new("npx")
                .args(["tsc", "--outDir", "build"])
                .current_dir(path)
                .output()?,
        };

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let combined = format!("{stderr}\n{stdout}");
        let errors = parse_ts_errors(&combined);

        if opts.test && output.status.success() {
            let test_out = self.run_test(path)?;
            let test_combined = String::from_utf8_lossy(&test_out.stderr).to_string();
            return Ok(BuildResult {
                success: test_out.status.success(),
                output: test_combined,
                errors,
            });
        }

        if opts.run && output.status.success() {
            let run_out = self.run_script(path)?;
            let run_stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
            return Ok(BuildResult {
                success: run_out.status.success(),
                output: run_stdout,
                errors,
            });
        }

        Ok(BuildResult {
            success: output.status.success(),
            output: combined,
            errors,
        })
    }

    fn lint(&self, path: &Path, opts: &LintOpts) -> Result<LintResult> {
        let output = match self.linter.as_str() {
            "biome" => {
                let mut args = vec!["biome"];
                if opts.fix {
                    args.extend_from_slice(&["check", "--fix", "."]);
                } else {
                    args.extend_from_slice(&["check", "."]);
                }
                Command::new("npx").args(&args).current_dir(path).output()?
            }
            "oxlint" => {
                let mut args = vec!["oxlint"];
                if opts.fix {
                    args.push("--fix");
                }
                args.push(".");
                Command::new("npx").args(&args).current_dir(path).output()?
            }
            _ => {
                let mut args = vec!["eslint"];
                if opts.fix {
                    args.push("--fix");
                }
                args.extend_from_slice(&["--format", "json", "."]);
                Command::new("npx").args(&args).current_dir(path).output()?
            }
        };

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostics = parse_ts_errors(&combined);

        Ok(LintResult {
            success: output.status.success(),
            diagnostics,
        })
    }

    fn clean(&self, path: &Path) -> Result<()> {
        let build_dir = path.join("build");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        let nm = path.join("node_modules");
        if nm.exists() {
            std::fs::remove_dir_all(&nm)?;
        }
        Ok(())
    }
}

impl TypeScriptPlugin {
    fn install_deps(&self, path: &Path) -> Result<()> {
        if !path.join("node_modules").exists() {
            let cmd = match self.package_manager.as_str() {
                "bun" => ("bun", vec!["install"]),
                "yarn" => ("yarn", vec!["install"]),
                "pnpm" => ("pnpm", vec!["install"]),
                _ => ("npm", vec!["install"]),
            };
            Command::new(cmd.0)
                .args(&cmd.1)
                .current_dir(path)
                .output()?;
        }
        Ok(())
    }

    fn run_test(&self, path: &Path) -> Result<std::process::Output> {
        Ok(match self.runtime.as_str() {
            "bun" => Command::new("bun")
                .args(["test"])
                .current_dir(path)
                .output()?,
            "deno" => Command::new("deno")
                .args(["test"])
                .current_dir(path)
                .output()?,
            _ => Command::new("npx")
                .args(["jest"])
                .current_dir(path)
                .output()?,
        })
    }

    fn run_script(&self, path: &Path) -> Result<std::process::Output> {
        Ok(match self.runtime.as_str() {
            "bun" => Command::new("bun")
                .args(["run", "src/index.ts"])
                .current_dir(path)
                .output()?,
            "deno" => Command::new("deno")
                .args(["run", "src/index.ts"])
                .current_dir(path)
                .output()?,
            _ => Command::new("node")
                .args(["build/index.js"])
                .current_dir(path)
                .output()?,
        })
    }
}

fn parse_ts_errors(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(.+\.(?:ts|tsx|js|jsx))\((\d+),(\d+)\):\s*(\w+)\s+(\w+):\s*(.+)")
        .unwrap();
    for cap in re.captures_iter(output) {
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: cap[5].to_string(),
            severity: if &cap[4] == "error" {
                Severity::Error
            } else {
                Severity::Warning
            },
            message: cap[6].to_string(),
            suggestion: None,
        });
    }

    let re2 = regex::Regex::new(r"(.+\.(?:ts|tsx|js|jsx)):(\d+):(\d+)\s*[-–]\s*(.+)").unwrap();
    for cap in re2.captures_iter(output) {
        if diags
            .iter()
            .any(|d| d.file == cap[1] && d.line == cap[2].parse().unwrap_or(0))
        {
            continue;
        }
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: "lint".into(),
            severity: Severity::Warning,
            message: cap[4].to_string(),
            suggestion: None,
        });
    }

    diags
}
