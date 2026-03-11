use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct ZigPlugin;

impl Plugin for ZigPlugin {
    fn language(&self) -> Language {
        Language::Zig
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("build.zig").exists()
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let build_dir = path.join("build");
        std::fs::create_dir_all(&build_dir)?;

        let mut args = vec!["build"];
        if opts.release {
            args.extend_from_slice(&["-Doptimize=ReleaseFast"]);
        }
        args.extend_from_slice(&["--prefix", "build"]);

        let output = Command::new("zig").args(&args).current_dir(path).output()?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let errors = parse_zig_errors(&stderr);

        if opts.test && output.status.success() {
            let test_out = Command::new("zig")
                .args(["build", "test"])
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
            let bin_path = path.join("build").join("bin");
            if let Ok(entries) = std::fs::read_dir(&bin_path) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        let run_out = Command::new(entry.path()).current_dir(path).output()?;
                        let stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
                        return Ok(BuildResult {
                            success: run_out.status.success(),
                            output: stdout,
                            errors,
                        });
                    }
                }
            }
        }

        Ok(BuildResult {
            success: output.status.success(),
            output: stderr,
            errors,
        })
    }

    fn lint(&self, path: &Path, _opts: &LintOpts) -> Result<LintResult> {
        let output = Command::new("zig")
            .args(["build"])
            .current_dir(path)
            .output()?;

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let diagnostics = parse_zig_errors(&stderr);

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
        let zig_cache = path.join("zig-cache");
        if zig_cache.exists() {
            std::fs::remove_dir_all(&zig_cache)?;
        }
        let zig_out = path.join("zig-out");
        if zig_out.exists() {
            std::fs::remove_dir_all(&zig_out)?;
        }
        Ok(())
    }
}

fn parse_zig_errors(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(.+\.zig):(\d+):(\d+):\s*(error|warning|note):\s*(.+)").unwrap();
    for cap in re.captures_iter(output) {
        let severity = match &cap[4] {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            _ => Severity::Info,
        };
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: "compiler".into(),
            severity,
            message: cap[5].to_string(),
            suggestion: None,
        });
    }
    diags
}
