use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "poc", about = "plyght's own compiler — polyglot build tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(long, global = true, help = "Use release/optimized build")]
    pub release: bool,

    #[arg(long, global = true, help = "Override TS runtime (bun|node|deno)")]
    pub runtime: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Override package manager (bun|npm|yarn|pnpm)"
    )]
    pub package_manager: Option<String>,

    #[arg(long, global = true, help = "Override C/C++ compiler (clang|gcc)")]
    pub compiler: Option<String>,

    #[arg(long, global = true, help = "Override Rust linker (mold|lld|default)")]
    pub linker: Option<String>,

    #[arg(long, global = true, help = "Override linter for relevant language")]
    pub linter: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Override Python runner (uv|pip|poetry|pdm)"
    )]
    pub runner: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Build {
        #[arg(short, long, help = "Run tests after build")]
        test: bool,
        #[arg(short, long, help = "Run the built artifact")]
        run: bool,
        #[arg(long, help = "Run linter after build")]
        lint: bool,
        #[arg(long, help = "Clean before building")]
        clean: bool,
        #[arg(long, help = "AI-fix errors after build/lint")]
        fix: bool,
    },
    Lint {
        #[arg(long, help = "Auto-fix lint issues")]
        fix: bool,
    },
    Clean,
    Fix {
        #[arg(long, help = "AI provider override")]
        provider: Option<String>,
        #[arg(long, help = "AI model override")]
        model: Option<String>,
    },
}
