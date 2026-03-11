use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct GoPlugin;

impl Plugin for GoPlugin {
    fn language(&self) -> Language {
        Language::Go
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("go.mod").exists()
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let build_dir = path.join("build");
        std::fs::create_dir_all(&build_dir)?;

        let output_path = build_dir.join("main");
        let mut args = vec!["build", "-o", output_path.to_str().unwrap_or("build/main")];

        let ldflags;
        if opts.release {
            ldflags = "-s -w".to_string();
            args.extend_from_slice(&["-ldflags", &ldflags]);
        }

        args.push("./...");

        let output = Command::new("go").args(&args).current_dir(path).output()?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let errors = parse_go_errors(&stderr);

        if opts.test && output.status.success() {
            let test_out = Command::new("go")
                .args(["test", "./..."])
                .current_dir(path)
                .output()?;
            let test_stderr = String::from_utf8_lossy(&test_out.stderr).to_string();
            return Ok(BuildResult {
                success: test_out.status.success(),
                output: test_stderr,
                errors,
            });
        }

        if opts.run && output.status.success() {
            let run_out = Command::new(&output_path).current_dir(path).output()?;
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
        let output = Command::new("go")
            .args(["vet", "./..."])
            .current_dir(path)
            .output()?;

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let mut diagnostics = parse_go_errors(&stderr);

        if let Ok(staticcheck) = Command::new("staticcheck")
            .args(["./..."])
            .current_dir(path)
            .output()
        {
            let sc_out = String::from_utf8_lossy(&staticcheck.stdout).to_string();
            diagnostics.extend(parse_go_errors(&sc_out));
        }

        let _ = opts;

        Ok(LintResult {
            success: output.status.success() && diagnostics.is_empty(),
            diagnostics,
        })
    }

    fn clean(&self, path: &Path) -> Result<()> {
        Command::new("go")
            .args(["clean"])
            .current_dir(path)
            .output()?;
        let build_dir = path.join("build");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        Ok(())
    }
}

fn parse_go_errors(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(.+\.go):(\d+):(\d+):\s*(.+)").unwrap();
    for cap in re.captures_iter(output) {
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: "go-vet".into(),
            severity: Severity::Error,
            message: cap[4].to_string(),
            suggestion: None,
        });
    }
    diags
}
