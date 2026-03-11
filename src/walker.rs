use crate::types::{DetectedProject, Language, Plugin};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MANIFEST_FILES: &[(&str, Language)] = &[
    ("Cargo.toml", Language::Rust),
    ("go.mod", Language::Go),
    ("package.json", Language::TypeScript),
    ("CMakeLists.txt", Language::C),
    ("Makefile", Language::C),
    ("pyproject.toml", Language::Python),
    ("build.zig", Language::Zig),
];

pub fn detect_projects(root: &Path, plugins: &[Box<dyn Plugin>]) -> Vec<DetectedProject> {
    let mut projects = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != "build"
                && name != "zig-cache"
                && name != "__pycache__"
                && name != ".venv"
                && name != "venv"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy();
        let parent = match entry.path().parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };

        for &(manifest, lang) in MANIFEST_FILES {
            if file_name == manifest && !seen.contains(&parent) {
                let plugin_match = plugins
                    .iter()
                    .any(|p| p.language() == lang && p.detect(&parent));
                if plugin_match {
                    seen.insert(parent.clone());
                    projects.push(DetectedProject {
                        path: parent.clone(),
                        language: lang,
                    });
                }
            }
        }
    }

    projects
}

pub fn watch_projects(
    _root: &Path,
    plugins: &[Box<dyn Plugin>],
    projects: &[DetectedProject],
    _config: &crate::config::PocConfig,
    test: bool,
    lint: bool,
) {
    use colored::Colorize;
    use std::time::{Duration, Instant};

    println!(
        "{} watching {} project(s) for changes...",
        "poc:".bold(),
        projects.len()
    );
    println!("  {} press Ctrl+C to stop", "·".dimmed());

    let mut last_modified = std::collections::HashMap::new();
    for proj in projects {
        if let Ok(meta) = std::fs::metadata(&proj.path) {
            if let Ok(modified) = meta.modified() {
                last_modified.insert(proj.path.clone(), modified);
            }
        }
    }

    loop {
        std::thread::sleep(Duration::from_secs(2));

        let mut changed_projects = Vec::new();
        for proj in projects {
            if has_changes(&proj.path, last_modified.get(&proj.path)) {
                changed_projects.push(proj.clone());
                if let Ok(meta) = std::fs::metadata(&proj.path) {
                    if let Ok(modified) = meta.modified() {
                        last_modified.insert(proj.path.clone(), modified);
                    }
                }
            }
        }

        if changed_projects.is_empty() {
            continue;
        }

        println!(
            "\n{} changes detected in {} project(s)",
            "poc:".cyan().bold(),
            changed_projects.len()
        );
        let start = Instant::now();

        let ordered = crate::orchestrator::resolve_build_order(&changed_projects);
        let opts = crate::types::BuildOpts {
            release: false,
            test,
            run: false,
        };
        let results = crate::orchestrator::run_build(&ordered, plugins, &opts);
        crate::orchestrator::print_build_results(&results);

        if lint {
            let lint_opts = crate::types::LintOpts { fix: false };
            let lint_results = crate::orchestrator::run_lint(&ordered, plugins, &lint_opts);
            crate::orchestrator::print_lint_results(&lint_results);
        }

        let elapsed = start.elapsed();
        println!("{} done in {:.1}s", "poc:".bold(), elapsed.as_secs_f64());
    }
}

fn has_changes(project_path: &Path, last: Option<&std::time::SystemTime>) -> bool {
    let walker = WalkDir::new(project_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != "build"
                && name != "zig-cache"
                && name != "__pycache__"
                && name != ".venv"
                && name != "venv"
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if let Some(prev) = last {
                    if modified > *prev {
                        return true;
                    }
                }
            }
        }
    }
    false
}
