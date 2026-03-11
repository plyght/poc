use crate::types::*;
use colored::Colorize;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
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
) -> Vec<(DetectedProject, anyhow::Result<BuildResult>)> {
    let independent = find_independent_groups(projects);

    let mut results = Vec::new();
    for group in independent {
        let group_results: Vec<_> = group
            .par_iter()
            .filter_map(|proj| {
                let plugin = plugins.iter().find(|p| p.language() == proj.language)?;
                if is_cached(proj) {
                    println!(
                        "{} {} ({}) (cached)",
                        "skipping".dimmed(),
                        proj.path.display(),
                        proj.language
                    );
                    return Some((
                        proj.clone(),
                        Ok(BuildResult {
                            success: true,
                            output: "cached".into(),
                            errors: vec![],
                        }),
                    ));
                }
                println!(
                    "{} {} ({})",
                    "building".green().bold(),
                    proj.path.display(),
                    proj.language
                );
                let result = plugin.build(&proj.path, opts);
                if let Ok(ref r) = result {
                    if r.success {
                        update_cache(proj);
                    }
                }
                Some((proj.clone(), result))
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
            println!(
                "{} {} ({})",
                "linting".cyan().bold(),
                proj.path.display(),
                proj.language
            );
            let result = plugin.lint(&proj.path, opts);
            Some((proj.clone(), result))
        })
        .collect()
}

pub fn run_clean(projects: &[DetectedProject], plugins: &[Box<dyn Plugin>]) {
    for proj in projects {
        if let Some(plugin) = plugins.iter().find(|p| p.language() == proj.language) {
            println!(
                "{} {} ({})",
                "cleaning".yellow().bold(),
                proj.path.display(),
                proj.language
            );
            if let Err(e) = plugin.clean(&proj.path) {
                eprintln!("  {} {e}", "error:".red());
            }
        }
    }
}

pub fn print_build_results(results: &[(DetectedProject, anyhow::Result<BuildResult>)]) {
    println!("\n{}", "── build results ──".bold());
    for (proj, result) in results {
        match result {
            Ok(r) if r.success => {
                println!(
                    "  {} {} ({})",
                    "✓".green(),
                    proj.path.display(),
                    proj.language
                );
            }
            Ok(r) => {
                println!(
                    "  {} {} ({})",
                    "✗".red(),
                    proj.path.display(),
                    proj.language
                );
                for err in &r.errors {
                    println!(
                        "    {}:{}:{} {}",
                        err.file,
                        err.line,
                        err.col,
                        err.message.dimmed()
                    );
                }
                if r.errors.is_empty() && !r.output.is_empty() {
                    for line in r.output.lines().take(10) {
                        println!("    {}", line.dimmed());
                    }
                }
            }
            Err(e) => {
                println!("  {} {} — {e}", "✗".red(), proj.path.display());
            }
        }
    }
}

pub fn print_lint_results(results: &[(DetectedProject, anyhow::Result<LintResult>)]) {
    let mut total = 0usize;
    let mut errors = 0usize;
    let mut warnings = 0usize;

    println!("\n{}", "── lint results ──".bold());
    for (proj, result) in results {
        match result {
            Ok(r) => {
                let diag_count = r.diagnostics.len();
                total += diag_count;
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
                errors += err_count;
                warnings += warn_count;

                if diag_count == 0 {
                    println!(
                        "  {} {} ({})",
                        "✓".green(),
                        proj.path.display(),
                        proj.language
                    );
                } else {
                    println!(
                        "  {} {} ({}) — {} issues",
                        if err_count > 0 {
                            "✗".red()
                        } else {
                            "⚠".yellow()
                        },
                        proj.path.display(),
                        proj.language,
                        diag_count
                    );
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
                println!("  {} {} — {e}", "✗".red(), proj.path.display());
            }
        }
    }

    let exit_status = if errors > 0 {
        "✗".red()
    } else if warnings > 0 {
        "⚠".yellow()
    } else {
        "✓".green()
    };
    println!(
        "\n  {} {} total, {} errors, {} warnings",
        exit_status, total, errors, warnings
    );
}

pub fn collect_all_diagnostics(
    build_results: &[(DetectedProject, anyhow::Result<BuildResult>)],
    lint_results: &[(DetectedProject, anyhow::Result<LintResult>)],
) -> Vec<(std::path::PathBuf, LintDiagnostic)> {
    let mut all = Vec::new();

    for (proj, result) in build_results {
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

pub fn has_failures(results: &[(DetectedProject, anyhow::Result<BuildResult>)]) -> bool {
    results.iter().any(|(_, r)| match r {
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
    println!("\n{}", "── project status ──".bold());
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

    println!("\n{}", "── toolchain ──".bold());
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

    println!("\n{}", "── dependency order ──".bold());
    for (i, proj) in projects.iter().enumerate() {
        println!(
            "  {}. {} ({})",
            i + 1,
            proj.path.file_name().unwrap_or_default().to_string_lossy(),
            proj.language
        );
    }

    println!("\n{}", "── lint config ──".bold());
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
        println!("\n{}", "── dependency graph ──".bold());
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
                    "← (no dependencies)".dimmed()
                );
            } else {
                println!(
                    "  {} ({}) {} {}",
                    name,
                    proj.language,
                    "←".dimmed(),
                    deps.join(", ")
                );
            }
        }
    }
}

pub fn print_json_build_results(results: &[(DetectedProject, anyhow::Result<BuildResult>)]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|(proj, result)| match result {
            Ok(r) => serde_json::json!({
                "path": proj.path.display().to_string(),
                "language": proj.language.to_string(),
                "success": r.success,
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
