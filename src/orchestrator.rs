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
                println!(
                    "{} {} ({})",
                    "building".green().bold(),
                    proj.path.display(),
                    proj.language
                );
                let result = plugin.build(&proj.path, opts);
                Some((proj.clone(), result))
            })
            .collect();
        results.extend(group_results);
    }
    results
}

fn find_independent_groups(projects: &[DetectedProject]) -> Vec<Vec<DetectedProject>> {
    let mut groups: Vec<Vec<DetectedProject>> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();

    for (i, proj) in projects.iter().enumerate() {
        if seen.contains(&i) {
            continue;
        }
        let mut group = vec![proj.clone()];
        seen.insert(i);

        for (j, other) in projects.iter().enumerate().skip(i + 1) {
            if seen.contains(&j) {
                continue;
            }
            if !project_depends_on(other, proj) && !project_depends_on(proj, other) {
                group.push(other.clone());
                seen.insert(j);
            }
        }
        groups.push(group);
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
