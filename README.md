<div align='center'>
    <h3>poc</h3>
    <p>Polyglot build tool that detects, orchestrates, and AI-fixes projects across six languages</p>
    <br/>
    <br/>
</div>

plyght's own compiler. Drop into any directory containing Rust, Go, TypeScript, C/C++, Python, or Zig projects and poc will discover them, resolve inter-project dependencies via topological sort, build independent projects in parallel, lint with your preferred toolchain, and optionally feed errors to an LLM for automated fixes with rollback safety.

## Features

- **Polyglot Detection**: Walks the directory tree and identifies projects by manifest files (Cargo.toml, go.mod, package.json, CMakeLists.txt, pyproject.toml, build.zig)
- **Dependency-Aware Ordering**: Parses manifests for path/local dependencies and topologically sorts the build graph, with cycle detection fallback
- **Parallel Builds**: Independent projects within the dependency graph are built concurrently via Rayon using proper topological-level grouping
- **Build Caching**: Hashes source files and skips rebuilds when nothing has changed
- **Unified Linting**: Delegates to per-language linters (Clippy, golangci-lint, Biome, clang-tidy, Ruff, zig build) through a common plugin interface
- **AI-Powered Fixes**: Sends build/lint diagnostics to Ollama, Anthropic, or any OpenAI-compatible endpoint; batches errors per-file for efficient LLM usage; rolls back garbage responses
- **Watch Mode**: Polls for file changes and automatically rebuilds/lints on save
- **Plugin Architecture**: Each language is a `Plugin` trait implementation, making new languages a single-file addition
- **Configurable Toolchains**: Override runtimes, compilers, linkers, package managers, and linters via TOML config or CLI flags
- **JSON Output**: Machine-readable output for CI/CD integration
- **Shell Completions**: Auto-generated completions for Bash, Zsh, Fish, PowerShell, and Elvish
- **Config Validation**: Warns on unknown sections and invalid provider names in poc.toml
- **Dependency Graph Visualization**: ASCII or Graphviz DOT output of the project dependency graph

## Supported Languages

| Language | Manifest | Compiler/Runtime | Linter | Build System |
|---|---|---|---|---|
| Rust | `Cargo.toml` | cargo | clippy | cargo |
| Go | `go.mod` | go | golangci-lint | go build |
| TypeScript | `package.json` | bun/node/deno | biome/eslint | bun/npm/yarn/pnpm |
| C/C++ | `CMakeLists.txt`, `Makefile` | clang/gcc | clang-tidy | cmake/make |
| Python | `pyproject.toml` | uv/pip/poetry/pdm | ruff | uv/pip/poetry/pdm |
| Zig | `build.zig` | zig | zig build | zig build |

## Install

```bash
git clone https://github.com/plyght/poc.git
cd poc
cargo build --release
sudo cp target/release/poc /usr/local/bin/
```

## Usage

```bash
poc build              # detect and build all projects
poc build --test       # build then run tests
poc build --run        # build then run the artifact
poc build --lint       # build then lint
poc build --clean      # clean before building
poc build --fix        # build, lint, then AI-fix errors
poc build --release    # optimized build

poc test               # run tests across all projects
poc test --filter foo  # filter tests by name pattern

poc lint               # lint all detected projects
poc lint --fix         # auto-fix lint issues

poc clean              # remove build artifacts

poc fix                # build + lint + AI-fix in one pass
poc fix --provider anthropic --model claude-sonnet-4-20250514
poc fix --max-fixes 5  # limit to fixing 5 files

poc init               # generate .poc/config.toml from detected projects
poc status             # show project status, toolchains, and config
poc graph              # print dependency graph (ASCII)
poc graph --dot        # print dependency graph (Graphviz DOT)

poc watch              # rebuild on file changes
poc watch --test       # rebuild and test on changes
poc watch --lint       # rebuild and lint on changes

poc completions bash   # generate shell completions
poc completions zsh
poc completions fish
```

Global flags apply to all subcommands:

```bash
poc --runtime deno build          # override TS runtime
poc --package-manager pnpm build  # override TS package manager
poc --compiler gcc build          # override C/C++ compiler
poc --linker mold build           # override Rust linker
poc --runner poetry build         # override Python runner
poc --linter eslint lint          # override linter
poc --json build                  # output results as JSON
poc --filter rust build           # only build Rust projects
poc --only ./my-app build         # only build specific project
poc --quiet build                 # suppress non-error output
poc --verbose build               # verbose output
```

## Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Build failure |
| 2 | Lint failure |
| 3 | AI fix failure |

## Configuration

poc loads configuration from `.poc/config.toml` in the project root, falling back to `~/.config/poc/config.toml`:

```toml
[ts]
runtime = "bun"
package_manager = "bun"

[python]
runner = "uv"

[c]
compiler = "clang"
build_system = "cmake"

[rust]
linker = "default"

[lint]
ts = "biome"
python = "ruff"
rust = "clippy"

[ai]
provider = "ollama"
model = "llama3"
endpoint = "http://0.0.0.0:11434"
```

The AI provider can be `ollama` (default, local), `anthropic` (requires `ANTHROPIC_API_KEY`), or any OpenAI-compatible endpoint (requires `OPENAI_API_KEY`).

Run `poc init` to auto-generate `.poc/config.toml` based on detected projects. Unknown config sections and invalid provider names will produce warnings. Build caches are stored in `.poc/cache/`. Add `.poc/` to your `.gitignore`.

## Architecture

```
src/
  main.rs           CLI entry point and command dispatch
  cli.rs            Argument parsing via clap derive
  config.rs         TOML config loading, generation, and validation
  types.rs          Core traits (Plugin) and shared types
  walker.rs         Recursive project discovery and watch mode
  orchestrator.rs   Dependency resolution, parallel execution, caching, result reporting
  ai.rs             LLM integration with batched-by-file fixing and rollback logic
  plugins/
    mod.rs          Plugin registry
    rust.rs         Rust plugin (cargo)
    go.rs           Go plugin (go build)
    typescript.rs   TypeScript plugin (bun/node/deno)
    c.rs            C/C++ plugin (cmake/make + clang/gcc)
    python.rs       Python plugin (uv/pip/poetry/pdm)
    zig.rs          Zig plugin (zig build)
```

The orchestrator builds a dependency graph from manifest analysis, groups projects by topological level for maximum parallelism, and dispatches each level to Rayon's thread pool. Successful builds are cached by content hash; unchanged projects are skipped on subsequent runs. Each plugin encapsulates detection, build, lint, and clean operations for its language. The AI fixer batches all errors per-file into a single LLM prompt, validates response quality, and rolls back suspicious fixes.

## Development

```bash
cargo build
cargo test
cargo clippy
```

Requires Rust 1.70+. Key dependencies: clap, clap_complete, rayon, reqwest (blocking), serde, toml, walkdir, regex, colored, dirs.

## License

MIT License
