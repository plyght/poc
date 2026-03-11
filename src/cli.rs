use clap::{CommandFactory, Parser, Subcommand};

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

    #[arg(long, global = true, help = "Output results as JSON")]
    pub json: bool,

    #[arg(
        long,
        global = true,
        help = "Filter by language (rust|go|typescript|c|python|zig)"
    )]
    pub filter: Option<String>,

    #[arg(long, global = true, help = "Only build specific project path")]
    pub only: Option<String>,

    #[arg(long, global = true, help = "Verbose output")]
    pub verbose: bool,

    #[arg(short, long, global = true, help = "Quiet mode — errors only")]
    pub quiet: bool,
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
        #[arg(long, help = "Maximum number of files to fix")]
        max_fixes: Option<usize>,
    },
    Init,
    Status,
    Test {
        #[arg(long, help = "Filter test by name pattern")]
        filter: Option<String>,
    },
    Watch {
        #[arg(long, help = "Run tests on change")]
        test: bool,
        #[arg(long, help = "Run linter on change")]
        lint: bool,
    },
    Graph {
        #[arg(long, help = "Output in DOT format")]
        dot: bool,
    },
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

impl Cli {
    pub fn generate_completions(shell: clap_complete::Shell) {
        clap_complete::generate(shell, &mut Self::command(), "poc", &mut std::io::stdout());
    }
}
