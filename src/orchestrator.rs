use crate::types::*;
use colored::Colorize;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub type BuildEntry = (DetectedProject, anyhow::Result<BuildResult>, bool, Duration);

pub fn resolve_build_order(projects: &[DetectedProject]) -> Vec<DetectedProject> {
    let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut in_degree: HashMap<usize, usize> = HashMap::new();

    for i in 0..projects.len() {
        graph.entry(i).or_default();
        in_degree.entry(i).or_insert(0);
    }

    for i in 0..projects.len() {
        for j in 0..projects.len() {
            if i == j {
                continue;
            }
            if project_depends_on(&projects[i], &projects[j]) {
                graph.entry(j).or_default().push(i);
                *in_degree.entry(i).or_insert(0) += 1;
            }
        }
    }

    let mut queue: Vec<usize> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&idx, _)| idx)
        .collect();
    queue.sort();

    let mut ordered = Vec::new();
    while let Some(idx) = queue.pop() {
        ordered.push(idx);
        if let Some(dependents) = graph.get(&idx) {
            for &dep in dependents {
                if let Some(deg) = in_degree.get_mut(&dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(dep);
                    }
                }
            }
        }
    }

    if ordered.len() != projects.len() {
        eprintln!(
            "{} circular dependency detected, falling back to discovery order",
            "warning:".yellow()
        );
        return projects.to_vec();
    }

    ordered.iter().map(|&i| projects[i].clone()).collect()
}

fn project_depends_on(a: &DetectedProject, b: &DetectedProject) -> bool {
    let a_path = &a.path;
    let b_path = &b.path;

    match a.language {
        Language::Rust => {
            let cargo_toml = a_path.join("Cargo.toml");
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                if let Some(b_name) = b_path.file_name().and_then(|n| n.to_str()) {
                    if content.contains("path = ")
                        && content.contains(
                            &b_path
                                .strip_prefix(a_path.parent().unwrap_or(a_path))
                                .unwrap_or(b_path)
                                .display()
                                .to_string(),
                        )
                    {
                        return true;
                    }
                    if content.contains(b_name) {
                        let dep_sections = [
                            "[dependencies]",
                            "[dev-dependencies]",
                            "[build-dependencies]",
                        ];
                        for section in &dep_sections {
                            if let Some(idx) = content.find(section) {
                                let section_content = &content[idx..];
                                if let Some(end) = section_content[1..].find('[') {
                                    let slice = &section_content[..end + 1];
                                    if slice.contains(b_name) && slice.contains("path") {
                                        return true;
                                    }
                                } else if section_content.contains(b_name)
                                    && section_content.contains("path")
                                {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        Language::Go => {
            let go_mod = a_path.join("go.mod");
            if let Ok(content) = std::fs::read_to_string(&go_mod) {
                if content.contains("replace") {
                    if let Some(b_name) = b_path.file_name().and_then(|n| n.to_str()) {
                        if content.contains(b_name) {
                            return true;
                        }
                    }
                }
            }
        }
        Language::TypeScript => {
            let pkg_json = a_path.join("package.json");
            if let Ok(content) = std::fs::read_to_string(&pkg_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    for section in &["dependencies", "devDependencies"] {
                        if let Some(deps) = val.get(section).and_then(|d| d.as_object()) {
                            for (_, v) in deps {
                                if let Some(s) = v.as_str() {
                                    if s.starts_with("file:")
                                        || s.starts_with("link:")
                                        || s.starts_with("workspace:")
                                    {
                                        let dep_path = s
                                            .trim_start_matches("file:")
                                            .trim_start_matches("link:")
                                            .trim_start_matches("workspace:");
                                        let resolved = a_path.join(dep_path);
                                        if resolved == *b_path
                                            || resolved.canonicalize().ok()
                                                == b_path.canonicalize().ok()
                                        {
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Language::Python => {
            let pyproject = a_path.join("pyproject.toml");
            if let Ok(content) = std::fs::read_to_string(&pyproject) {
                if let Some(b_name) = b_path.file_name().and_then(|n| n.to_str()) {
                    if content.contains(b_name) && content.contains("path") {
                        return true;
                    }
                }
            }
        }
        Language::C => {
            let cmake = a_path.join("CMakeLists.txt");
            if let Ok(content) = std::fs::read_to_string(&cmake) {
                if let Some(b_name) = b_path.file_name().and_then(|n| n.to_str()) {
                    if content.contains("add_subdirectory") && content.contains(b_name) {
                        return true;
                    }
                }
            }
        }
        Language::Zig => {
            let build_zig = a_path.join("build.zig");
            if let Ok(content) = std::fs::read_to_string(&build_zig) {
                if let Some(b_name) = b_path.file_name().and_then(|n| n.to_str()) {
                    if content.contains("dependency") && content.contains(b_name) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn run_build(
    projects: &[DetectedProject],
    plugins: &[Box<dyn Plugin>],
    opts: &BuildOpts,
) -> Vec<BuildEntry> {
    let independent = find_independent_groups(projects);

    let mut results: Vec<BuildEntry> = Vec::new();
    for group in independent {
        let group_results: Vec<BuildEntry> = group
            .par_iter()
            .filter_map(|proj| {
                let plugin = plugins.iter().find(|p| p.language() == proj.language)?;
                if is_cached(proj) {
                    if opts.verbose {
                        if let Some(hash) = compute_project_hash(&proj.path) {
                            println!(
                                "cache hit {} ({}) hash={}",
                                proj.path.display(),
                                proj.language,
                                hash
                            );
                        }
                    }
                    return Some((
                        proj.clone(),
                        Ok(BuildResult {
                            success: true,
                            output: String::new(),
                            errors: vec![],
                        }),
                        true,
                        Duration::ZERO,
                    ));
                }
                println!("building {} ({})", proj.path.display(), proj.language);
                let t = Instant::now();
                let result = plugin.build(&proj.path, opts);
                let elapsed = t.elapsed();
                if let Ok(ref r) = result {
                    if r.success {
                        update_cache(proj);
                    }
                }
                Some((proj.clone(), result, false, elapsed))
            })
            .collect();
        results.extend(group_results);
    }
    results
}

fn find_independent_groups(projects: &[DetectedProject]) -> Vec<Vec<DetectedProject>> {
    let n = projects.len();
    if n == 0 {
        return Vec::new();
    }

    let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut in_degree: HashMap<usize, usize> = HashMap::new();

    for i in 0..n {
        graph.entry(i).or_default();
        in_degree.entry(i).or_insert(0);
    }

    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            if project_depends_on(&projects[i], &projects[j]) {
                graph.entry(j).or_default().push(i);
                *in_degree.entry(i).or_insert(0) += 1;
            }
        }
    }

    let mut groups: Vec<Vec<DetectedProject>> = Vec::new();
    let mut remaining: HashSet<usize> = (0..n).collect();

    while !remaining.is_empty() {
        let mut level: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|i| in_degree[i] == 0)
            .collect();
        level.sort();

        if level.is_empty() {
            let mut rest: Vec<DetectedProject> =
                remaining.iter().map(|&i| projects[i].clone()).collect();
            rest.sort_by_key(|p| p.path.clone());
            groups.push(rest);
            break;
        }

        let group: Vec<DetectedProject> = level.iter().map(|&i| projects[i].clone()).collect();
        groups.push(group);

        for &idx in &level {
            remaining.remove(&idx);
            if let Some(dependents) = graph.get(&idx) {
                for &dep in dependents {
                    if let Some(deg) = in_degree.get_mut(&dep) {
                        *deg -= 1;
                    }
                }
            }
        }
    }

    groups
}

pub fn run_lint(
    projects: &[DetectedProject],
    plugins: &[Box<dyn Plugin>],
    opts: &LintOpts,
) -> Vec<(DetectedProject, anyhow::Result<LintResult>)> {
    projects
        .par_iter()
        .filter_map(|proj| {
            let plugin = plugins.iter().find(|p| p.language() == proj.language)?;
            println!("linting {} ({})", proj.path.display(), proj.language);
            if opts.verbose {
                println!("  {} checking {}", "·".dimmed(), proj.language);
            }
            let result = plugin.lint(&proj.path, opts);
            Some((proj.clone(), result))
        })
        .collect()
}

pub fn run_clean(projects: &[DetectedProject], plugins: &[Box<dyn Plugin>]) {
    for proj in projects {
        if let Some(plugin) = plugins.iter().find(|p| p.language() == proj.language) {
            println!("cleaning {} ({})", proj.path.display(), proj.language);
            if let Err(e) = plugin.clean(&proj.path) {
                eprintln!("  {} {e}", "error:".red().bold());
            }
        }
    }
}

pub fn print_build_results(results: &[BuildEntry], elapsed: Duration, verbose: bool) {
    let mut built = 0usize;
    let mut cached = 0usize;
    let mut failed = 0usize;

    println!();
    for (proj, result, was_cached, proj_elapsed) in results {
        match result {
            Ok(r) if r.success && *was_cached => {
                cached += 1;
                println!(
                    "{} {} ({}) {}",
                    "+".green(),
                    proj.path.display(),
                    proj.language,
                    "(cached)".dimmed()
                );
            }
            Ok(r) if r.success => {
                built += 1;
                println!(
                    "{} {} ({}) {}",
                    "+".green(),
                    proj.path.display(),
                    proj.language,
                    format!("[{}ms]", proj_elapsed.as_millis()).dimmed()
                );
                if verbose && !r.output.is_empty() {
                    for line in r.output.lines() {
                        println!("    {}", line.dimmed());
                    }
                }
            }
            Ok(r) => {
                failed += 1;
                println!(
                    "{} {} ({}) -- build failed {}",
                    "x".red(),
                    proj.path.display(),
                    proj.language,
                    format!("[{}ms]", proj_elapsed.as_millis()).dimmed()
                );
                for err in &r.errors {
                    let sev = match err.severity {
                        Severity::Error => "error".red(),
                        Severity::Warning => "warning".yellow(),
                        Severity::Info => "info".blue(),
                        Severity::Hint => "hint".dimmed(),
                    };
                    println!(
                        "    {}:{}:{} {} [{}] {}",
                        err.file, err.line, err.col, sev, err.rule, err.message
                    );
                }
                if r.errors.is_empty() && !r.output.is_empty() {
                    let limit = if verbose { usize::MAX } else { 10 };
                    for line in r.output.lines().take(limit) {
                        println!("    {}", line.dimmed());
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!(
                    "{} {} ({}) -- {e}",
                    "x".red(),
                    proj.path.display(),
                    proj.language
                );
            }
        }
    }

    println!();
    let total_built = built + cached;
    if failed > 0 {
        println!(
            "{} built, {} cached, {} failed {}",
            built,
            cached,
            failed,
            format!("[{}ms]", elapsed.as_millis()).dimmed()
        );
    } else {
        println!(
            "{} built, {} cached {}",
            total_built,
            cached,
            format!("[{}ms]", elapsed.as_millis()).dimmed()
        );
    }
}

pub fn print_lint_results(
    results: &[(DetectedProject, anyhow::Result<LintResult>)],
    elapsed: Duration,
    verbose: bool,
) {
    let mut linted = 0usize;
    let mut total_errors = 0usize;
    let mut total_warnings = 0usize;

    println!();
    for (proj, result) in results {
        match result {
            Ok(r) => {
                linted += 1;
                let err_count = r
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
                    .count();
                let warn_count = r
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == Severity::Warning)
                    .count();
                total_errors += err_count;
                total_warnings += warn_count;

                if r.diagnostics.is_empty() {
                    println!(
                        "{} {} ({})",
                        "+".green(),
                        proj.path.display(),
                        proj.language
                    );
                } else {
                    println!(
                        "{} {} ({}) -- {} issue{}",
                        if err_count > 0 {
                            "x".red()
                        } else {
                            "!".yellow()
                        },
                        proj.path.display(),
                        proj.language,
                        r.diagnostics.len(),
                        if r.diagnostics.len() == 1 { "" } else { "s" }
                    );
                    let _ = verbose;
                    for d in &r.diagnostics {
                        let sev = match d.severity {
                            Severity::Error => "error".red(),
                            Severity::Warning => "warning".yellow(),
                            Severity::Info => "info".blue(),
                            Severity::Hint => "hint".dimmed(),
                        };
                        println!(
                            "    {}:{}:{} {} [{}] {}",
                            d.file, d.line, d.col, sev, d.rule, d.message
                        );
                        if let Some(ref s) = d.suggestion {
                            println!("      {} {s}", "fix:".green());
                        }
                    }
                }
            }
            Err(e) => {
                println!(
                    "{} {} ({}) -- {e}",
                    "x".red(),
                    proj.path.display(),
                    proj.language
                );
            }
        }
    }

    println!();
    println!(
        "{} linted, {} error{}, {} warning{} {}",
        linted,
        total_errors,
        if total_errors == 1 { "" } else { "s" },
        total_warnings,
        if total_warnings == 1 { "" } else { "s" },
        format!("[{}ms]", elapsed.as_millis()).dimmed()
    );
}

pub fn collect_all_diagnostics(
    build_results: &[BuildEntry],
    lint_results: &[(DetectedProject, anyhow::Result<LintResult>)],
) -> Vec<(std::path::PathBuf, LintDiagnostic)> {
    let mut all = Vec::new();

    for (proj, result, _cached, _elapsed) in build_results {
        if let Ok(r) = result {
            for d in &r.errors {
                all.push((proj.path.clone(), d.clone()));
            }
        }
    }
    for (proj, result) in lint_results {
        if let Ok(r) = result {
            for d in &r.diagnostics {
                all.push((proj.path.clone(), d.clone()));
            }
        }
    }

    all
}

pub fn has_failures(results: &[BuildEntry]) -> bool {
    results.iter().any(|(_, r, _, _)| match r {
        Ok(r) => !r.success,
        Err(_) => true,
    })
}

pub fn has_lint_failures(results: &[(DetectedProject, anyhow::Result<LintResult>)]) -> bool {
    results.iter().any(|(_, r)| match r {
        Ok(r) => !r.success,
        Err(_) => true,
    })
}

pub fn print_status(
    projects: &[DetectedProject],
    plugins: &[Box<dyn Plugin>],
    config: &crate::config::PocConfig,
) {
    println!("projects");
    for proj in projects {
        let plugin = plugins.iter().find(|p| p.language() == proj.language);
        let status = if plugin.is_some() {
            "ready".green()
        } else {
            "no plugin".red()
        };
        println!(
            "  {} {} ({}) [{}]",
            "·".dimmed(),
            proj.path.display(),
            proj.language,
            status
        );
    }

    println!();
    println!("toolchain");
    println!(
        "  {} ts runtime: {}, pm: {}",
        "·".dimmed(),
        config.ts.runtime,
        config.ts.package_manager
    );
    println!("  {} python runner: {}", "·".dimmed(), config.python.runner);
    println!(
        "  {} c compiler: {}, build: {}",
        "·".dimmed(),
        config.c.compiler,
        config.c.build_system
    );
    println!("  {} rust linker: {}", "·".dimmed(), config.rust.linker);
    println!(
        "  {} ai: {} ({})",
        "·".dimmed(),
        config.ai.provider,
        config.ai.model
    );

    println!();
    println!("dependency order");
    for (i, proj) in projects.iter().enumerate() {
        println!(
            "  {}. {} ({})",
            i + 1,
            proj.path.file_name().unwrap_or_default().to_string_lossy(),
            proj.language
        );
    }

    println!();
    println!("lint config");
    println!("  {} ts: {}", "·".dimmed(), config.lint.ts);
    println!("  {} python: {}", "·".dimmed(), config.lint.python);
    println!("  {} rust: {}", "·".dimmed(), config.lint.rust);
}

pub fn print_graph(projects: &[DetectedProject], dot: bool) {
    if dot {
        println!("digraph poc {{");
        println!("  rankdir=LR;");
        println!("  node [shape=box];");
        for proj in projects {
            let name = proj.path.file_name().unwrap_or_default().to_string_lossy();
            println!("  \"{}\" [label=\"{} ({})\"];", name, name, proj.language);
        }
        for (i, proj_a) in projects.iter().enumerate() {
            for proj_b in projects.iter().skip(i + 1) {
                if project_depends_on(proj_a, proj_b) {
                    let a = proj_a
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let b = proj_b
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    println!("  \"{}\" -> \"{}\";", a, b);
                } else if project_depends_on(proj_b, proj_a) {
                    let a = proj_a
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let b = proj_b
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    println!("  \"{}\" -> \"{}\";", b, a);
                }
            }
        }
        println!("}}");
    } else {
        println!("dependency graph");
        for proj in projects {
            let name = proj.path.file_name().unwrap_or_default().to_string_lossy();
            let mut deps = Vec::new();
            for other in projects {
                if project_depends_on(proj, other) {
                    deps.push(
                        other
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                }
            }
            if deps.is_empty() {
                println!(
                    "  {} ({}) {}",
                    name,
                    proj.language,
                    "<- (no dependencies)".dimmed()
                );
            } else {
                println!(
                    "  {} ({}) {} {}",
                    name,
                    proj.language,
                    "<-".dimmed(),
                    deps.join(", ")
                );
            }
        }
    }
}

pub fn print_json_build_results(results: &[BuildEntry]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|(proj, result, cached, elapsed)| match result {
            Ok(r) => serde_json::json!({
                "path": proj.path.display().to_string(),
                "language": proj.language.to_string(),
                "success": r.success,
                "cached": cached,
                "elapsed_ms": elapsed.as_millis(),
                "errors": r.errors.iter().map(|e| serde_json::json!({
                    "file": e.file, "line": e.line, "col": e.col,
                    "rule": e.rule, "severity": format!("{:?}", e.severity),
                    "message": e.message
                })).collect::<Vec<_>>()
            }),
            Err(e) => serde_json::json!({
                "path": proj.path.display().to_string(),
                "language": proj.language.to_string(),
                "success": false,
                "cached": cached,
                "elapsed_ms": elapsed.as_millis(),
                "error": e.to_string()
            }),
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&items).unwrap_or_default()
    );
}

pub fn print_json_lint_results(results: &[(DetectedProject, anyhow::Result<LintResult>)]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|(proj, result)| match result {
            Ok(r) => serde_json::json!({
                "path": proj.path.display().to_string(),
                "language": proj.language.to_string(),
                "success": r.success,
                "diagnostics": r.diagnostics.iter().map(|d| serde_json::json!({
                    "file": d.file, "line": d.line, "col": d.col,
                    "rule": d.rule, "severity": format!("{:?}", d.severity),
                    "message": d.message, "suggestion": d.suggestion
                })).collect::<Vec<_>>()
            }),
            Err(e) => serde_json::json!({
                "path": proj.path.display().to_string(),
                "language": proj.language.to_string(),
                "success": false,
                "error": e.to_string()
            }),
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&items).unwrap_or_default()
    );
}

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn compute_project_hash(project_path: &std::path::Path) -> Option<u64> {
    let mut hasher = DefaultHasher::new();
    let walker = walkdir::WalkDir::new(project_path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != "build"
                && name != "zig-cache"
                && name != "__pycache__"
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(content) = std::fs::read(entry.path()) {
            entry.path().hash(&mut hasher);
            content.hash(&mut hasher);
        }
    }
    Some(hasher.finish())
}

fn project_cache_dir(project_path: &std::path::Path) -> std::path::PathBuf {
    project_path.join(".poc").join("cache")
}

pub fn is_cached(project: &DetectedProject) -> bool {
    let hash = match compute_project_hash(&project.path) {
        Some(h) => h,
        None => return false,
    };
    let cache_file = project_cache_dir(&project.path).join("build.hash");
    match std::fs::read_to_string(&cache_file) {
        Ok(cached) => cached.trim() == hash.to_string(),
        Err(_) => false,
    }
}

pub fn update_cache(project: &DetectedProject) {
    if let Some(hash) = compute_project_hash(&project.path) {
        let dir = project_cache_dir(&project.path);
        let _ = std::fs::create_dir_all(&dir);
        let cache_file = dir.join("build.hash");
        let _ = std::fs::write(cache_file, hash.to_string());
    }
}
