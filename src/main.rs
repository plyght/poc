mod ai;
mod cli;
mod config;
mod orchestrator;
mod plugins;
mod types;
mod walker;

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;
use config::PocConfig;
use types::*;

const EXIT_BUILD_FAIL: i32 = 1;
const EXIT_LINT_FAIL: i32 = 2;
const EXIT_AI_FAIL: i32 = 3;

fn main() {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().expect("failed to get cwd");

    let mut config = PocConfig::load(&cwd).unwrap_or_default();
    if let Some(ref runtime) = cli.runtime {
        config.ts.runtime = runtime.clone();
    }
    if let Some(ref pm) = cli.package_manager {
        config.ts.package_manager = pm.clone();
    }
    if let Some(ref compiler) = cli.compiler {
        config.c.compiler = compiler.clone();
    }
    if let Some(ref linker) = cli.linker {
        config.rust.linker = linker.clone();
    }
    if let Some(ref runner) = cli.runner {
        config.python.runner = runner.clone();
    }
    if let Some(ref linter) = cli.linter {
        config.lint.ts = linter.clone();
        config.lint.python = linter.clone();
        config.lint.rust = linter.clone();
    }

    let config_warnings = config::validate_config(&cwd);
    if !cli.quiet {
        for w in &config_warnings {
            eprintln!("{} {w}", "warning:".yellow());
        }
    }

    if let Command::Completions { shell } = cli.command {
        Cli::generate_completions(shell);
        return;
    }

    let plugins = plugins::all_plugins(&config);
    let projects = walker::detect_projects(&cwd, &plugins);

    if projects.is_empty() {
        eprintln!(
            "{} no projects detected in {}",
            "poc:".bold(),
            cwd.display()
        );
        std::process::exit(EXIT_BUILD_FAIL);
    }

    if !cli.quiet {
        println!("{} detected {} project(s)", "poc:".bold(), projects.len());
        for p in &projects {
            println!("  {} {} ({})", "·".dimmed(), p.path.display(), p.language);
        }
    }

    let projects = if let Some(ref lang) = cli.filter {
        projects
            .into_iter()
            .filter(|p| p.language.to_string() == *lang)
            .collect()
    } else if let Some(ref path) = cli.only {
        let target = std::path::PathBuf::from(path)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(path));
        projects
            .into_iter()
            .filter(|p| p.path == target || p.path.starts_with(&target))
            .collect()
    } else {
        projects
    };

    let ordered = orchestrator::resolve_build_order(&projects);

    match cli.command {
        Command::Build {
            test,
            run,
            lint,
            clean,
            fix,
        } => {
            if clean {
                orchestrator::run_clean(&ordered, &plugins);
            }

            let opts = BuildOpts {
                release: cli.release,
                test,
                run,
            };
            let results = orchestrator::run_build(&ordered, &plugins, &opts);
            if cli.json {
                orchestrator::print_json_build_results(&results);
            } else {
                orchestrator::print_build_results(&results);
            }

            if lint || fix {
                let lint_opts = LintOpts { fix: false };
                let lint_results = orchestrator::run_lint(&ordered, &plugins, &lint_opts);
                if cli.json {
                    orchestrator::print_json_lint_results(&lint_results);
                } else {
                    orchestrator::print_lint_results(&lint_results);
                }

                if fix {
                    let all_diags = orchestrator::collect_all_diagnostics(&results, &lint_results);
                    let fixer = ai::AiFixer::new(config.ai.clone());
                    match fixer.fix_diagnostics(&all_diags, &plugins) {
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("{} ai fix failed: {e}", "error:".red());
                            std::process::exit(EXIT_AI_FAIL);
                        }
                    }
                }
            }

            if orchestrator::has_failures(&results) {
                std::process::exit(EXIT_BUILD_FAIL);
            }
        }
        Command::Lint { fix } => {
            let opts = LintOpts { fix };
            let results = orchestrator::run_lint(&ordered, &plugins, &opts);
            if cli.json {
                orchestrator::print_json_lint_results(&results);
            } else {
                orchestrator::print_lint_results(&results);
            }

            if orchestrator::has_lint_failures(&results) {
                std::process::exit(EXIT_LINT_FAIL);
            }
        }
        Command::Clean => {
            orchestrator::run_clean(&ordered, &plugins);
            println!("{}", "done".green());
        }
        Command::Fix {
            provider,
            model,
            max_fixes,
        } => {
            let build_opts = BuildOpts {
                release: false,
                test: false,
                run: false,
            };
            let build_results = orchestrator::run_build(&ordered, &plugins, &build_opts);

            let lint_opts = LintOpts { fix: false };
            let lint_results = orchestrator::run_lint(&ordered, &plugins, &lint_opts);

            let all_diags = orchestrator::collect_all_diagnostics(&build_results, &lint_results);

            let fixer = ai::AiFixer::new(config.ai.clone())
                .with_overrides(provider.as_deref(), model.as_deref())
                .with_max_fixes(max_fixes);

            match fixer.fix_diagnostics(&all_diags, &plugins) {
                Ok(_report) => {}
                Err(e) => {
                    eprintln!("{} ai fix failed: {e}", "error:".red());
                    std::process::exit(EXIT_AI_FAIL);
                }
            }
        }
        Command::Init => match config::generate_config(&cwd, &projects) {
            Ok(_) => println!("{} initialized .poc/config.toml", "poc:".bold()),
            Err(e) => {
                eprintln!("{} {e}", "error:".red());
                std::process::exit(EXIT_BUILD_FAIL);
            }
        },
        Command::Status => {
            orchestrator::print_status(&ordered, &plugins, &config);
        }
        Command::Test { filter } => {
            let opts = BuildOpts {
                release: false,
                test: true,
                run: false,
            };
            let results = orchestrator::run_build(&ordered, &plugins, &opts);
            let _ = filter;
            orchestrator::print_build_results(&results);

            if orchestrator::has_failures(&results) {
                std::process::exit(EXIT_BUILD_FAIL);
            }
        }
        Command::Watch { test, lint } => {
            walker::watch_projects(&cwd, &plugins, &ordered, &config, test, lint);
        }
        Command::Graph { dot } => {
            orchestrator::print_graph(&ordered, dot);
        }
        Command::Completions { .. } => {}
    }
}
