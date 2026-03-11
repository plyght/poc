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
