use crate::types::{DetectedProject, Language, Plugin};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
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
    root: &Path,
    plugins: &[Box<dyn Plugin>],
    projects: &[DetectedProject],
    _config: &crate::config::PocConfig,
    test: bool,
    lint: bool,
) {
    println!(
        "watching {} project{} — press Ctrl+C to stop",
        projects.len(),
        if projects.len() == 1 { "" } else { "s" }
    );

    if try_notify_watch(root, plugins, projects, test, lint).is_err() {
        poll_watch(root, plugins, projects, test, lint);
    }
}

fn run_and_report(
    projects: &[DetectedProject],
    plugins: &[Box<dyn Plugin>],
    test: bool,
    lint: bool,
) {
    println!();
    let ordered = crate::orchestrator::resolve_build_order(projects);
    let opts = crate::types::BuildOpts {
        release: false,
        test,
        run: false,
        verbose: false,
        filter: None,
    };
    let build_start = Instant::now();
    let results = crate::orchestrator::run_build(&ordered, plugins, &opts);
    let build_elapsed = build_start.elapsed();
    crate::orchestrator::print_build_results(&results, build_elapsed, false);

    if lint {
        let lint_opts = crate::types::LintOpts {
            fix: false,
            verbose: false,
        };
        let lint_start = Instant::now();
        let lint_results = crate::orchestrator::run_lint(&ordered, plugins, &lint_opts);
        let lint_elapsed = lint_start.elapsed();
        crate::orchestrator::print_lint_results(&lint_results, lint_elapsed, false);
    }
}

fn try_notify_watch(
    root: &Path,
    plugins: &[Box<dyn Plugin>],
    projects: &[DetectedProject],
    test: bool,
    lint: bool,
) -> anyhow::Result<()> {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    loop {
        match rx.recv() {
            Ok(Ok(event)) => {
                let relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if !relevant {
                    continue;
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => break,
        }

        let debounce_end = Instant::now() + Duration::from_millis(300);
        loop {
            let remaining = debounce_end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(_) => {}
                Err(_) => break,
            }
        }

        run_and_report(projects, plugins, test, lint);
    }

    Ok(())
}

fn poll_watch(
    _root: &Path,
    plugins: &[Box<dyn Plugin>],
    projects: &[DetectedProject],
    test: bool,
    lint: bool,
) {
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

        run_and_report(&changed_projects, plugins, test, lint);
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
