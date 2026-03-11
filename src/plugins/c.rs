use crate::config::PocConfig;
use crate::types::*;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub struct CPlugin {
    compiler: String,
    build_system: String,
}

impl CPlugin {
    pub fn new(config: &PocConfig) -> Self {
        Self {
            compiler: config.c.compiler.clone(),
            build_system: config.c.build_system.clone(),
        }
    }
}

impl Plugin for CPlugin {
    fn language(&self) -> Language {
        Language::C
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("CMakeLists.txt").exists() || path.join("Makefile").exists()
    }

    fn build(&self, path: &Path, opts: &BuildOpts) -> Result<BuildResult> {
        let build_dir = path.join("build");
        std::fs::create_dir_all(&build_dir)?;

        let output = match self.build_system.as_str() {
            "cmake" => self.cmake_build(path, opts)?,
            "meson" => self.meson_build(path, opts)?,
            _ => self.make_build(path, opts)?,
        };

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let errors = parse_c_errors(&stderr);

        Ok(BuildResult {
            success: output.status.success(),
            output: stderr,
            errors,
        })
    }

    fn lint(&self, path: &Path, _opts: &LintOpts) -> Result<LintResult> {
        let output = Command::new("cppcheck")
            .args([
                "--enable=all",
                "--template={file}:{line}:{column}: {severity}: {message} [{id}]",
                ".",
            ])
            .current_dir(path)
            .output();

        match output {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let diagnostics = parse_cppcheck(&stderr);
                Ok(LintResult {
                    success: diagnostics.iter().all(|d| d.severity != Severity::Error),
                    diagnostics,
                })
            }
            Err(_) => {
                let build_result = self.build(
                    path,
                    &BuildOpts {
                        release: false,
                        test: false,
                        run: false,
                    },
                )?;
                Ok(LintResult {
                    success: build_result.success,
                    diagnostics: build_result.errors,
                })
            }
        }
    }

    fn clean(&self, path: &Path) -> Result<()> {
        let build_dir = path.join("build");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        Ok(())
    }
}

impl CPlugin {
    fn cmake_build(&self, path: &Path, opts: &BuildOpts) -> Result<std::process::Output> {
        let build_type = if opts.release { "Release" } else { "Debug" };
        Command::new("cmake")
            .args([
                "-B",
                "build",
                &format!("-DCMAKE_BUILD_TYPE={build_type}"),
                &format!("-DCMAKE_C_COMPILER={}", self.compiler),
                &format!("-DCMAKE_CXX_COMPILER={}++", self.compiler),
            ])
            .current_dir(path)
            .output()?;

        Ok(Command::new("cmake")
            .args(["--build", "build", "--parallel"])
            .current_dir(path)
            .output()?)
    }

    fn make_build(&self, path: &Path, opts: &BuildOpts) -> Result<std::process::Output> {
        let mut args = vec![
            format!("CC={}", self.compiler),
            format!("CXX={}++", self.compiler),
        ];
        if opts.release {
            args.push("RELEASE=1".into());
        }
        let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Ok(Command::new("make")
            .args(&str_args)
            .current_dir(path)
            .output()?)
    }

    fn meson_build(&self, path: &Path, opts: &BuildOpts) -> Result<std::process::Output> {
        let build_type = if opts.release { "release" } else { "debug" };
        if !path.join("build").join("build.ninja").exists() {
            Command::new("meson")
                .args(["setup", "build", &format!("--buildtype={build_type}")])
                .current_dir(path)
                .output()?;
        }
        Ok(Command::new("meson")
            .args(["compile", "-C", "build"])
            .current_dir(path)
            .output()?)
    }
}

fn parse_c_errors(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re =
        regex::Regex::new(r"(.+\.(?:c|cpp|h|hpp)):(\d+):(\d+):\s*(error|warning):\s*(.+)").unwrap();
    for cap in re.captures_iter(output) {
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: "compiler".into(),
            severity: if &cap[4] == "error" {
                Severity::Error
            } else {
                Severity::Warning
            },
            message: cap[5].to_string(),
            suggestion: None,
        });
    }
    diags
}

fn parse_cppcheck(output: &str) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();
    let re = regex::Regex::new(r"(.+):(\d+):(\d+):\s*(\w+):\s*(.+)\s*\[(\w+)\]").unwrap();
    for cap in re.captures_iter(output) {
        let severity = match &cap[4] {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            "style" | "performance" | "portability" => Severity::Info,
            _ => Severity::Hint,
        };
        diags.push(LintDiagnostic {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            col: cap[3].parse().unwrap_or(0),
            rule: cap[6].to_string(),
            severity,
            message: cap[5].to_string(),
            suggestion: None,
        });
    }
    diags
}
