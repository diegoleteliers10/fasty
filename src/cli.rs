use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct CliOptions {
    pub working_dir: Option<PathBuf>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub title: Option<String>,
}

impl CliOptions {
    pub fn parse() -> Self {
        Self::from_iter(std::env::args().skip(1)).unwrap_or_default()
    }

    pub fn from_iter<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut opts = Self::default();
        let mut iter = args.into_iter();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-v" | "--version" => {
                    println!("fastty {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "-h" | "--help" => {
                    println!(
                        "Fastty - GPU-accelerated terminal emulator\n\n\
                        USAGE:\n    \
                        fastty [OPTIONS] [-- <CMD>...]\n    \
                        fastty sessions\n    \
                        fastty attach <session-id>\n    \
                        fastty gateway [--port <PORT>]\n\n\
                        OPTIONS:\n    \
                        -d, --dir, --working-directory <DIR>  Set initial working directory\n    \
                        -t, --title <TITLE>                   Set initial tab/window title\n    \
                        -e, --exec, --command <CMD...>        Execute command and arguments\n    \
                        -v, --version                         Print version information\n    \
                        -h, --help                            Print this help message\n\n\
                        SUBCOMMANDS (talk to an already running fastty over its local\n    \
                        IPC daemon -- see docs/daemon-protocol.md):\n    \
                        sessions [--watch] [--wait[=SECS]] List live tabs/splits (or stream\n    \
                                                           changes with --watch)\n    \
                        attach <id> [--read-only] [--wait[=SECS]]\n    \
                                                           Attach interactively; --wait retries\n    \
                                                           until fastty/that session shows up\n    \
                        gateway [--port <PORT>] [--host <ADDR>]\n    \
                                                           Serve the embedded web/Wasm client and\n    \
                                                           bridge WebSocket traffic to the daemon\n\n\
                        EXAMPLES:\n    \
                        fastty --title \"Dev Server\" -d ~/api -e bun run dev\n    \
                        fastty -d /tmp\n    \
                        fastty -- htop\n    \
                        fastty sessions --watch\n    \
                        fastty attach 1 --read-only\n    \
                        fastty attach 1 --wait=30\n    \
                        fastty gateway --port 8765"
                    );
                    std::process::exit(0);
                }
                "-d" | "--dir" | "--working-directory" => {
                    if let Some(val) = iter.next() {
                        let expanded = if val.starts_with("~/") {
                            dirs::home_dir()
                                .map(|h| h.join(&val[2..]))
                                .unwrap_or_else(|| PathBuf::from(&val))
                        } else {
                            PathBuf::from(&val)
                        };
                        opts.working_dir = Some(expanded);
                    } else {
                        return Err("Missing argument for -d / --dir".to_string());
                    }
                }
                "-t" | "--title" => {
                    if let Some(val) = iter.next() {
                        opts.title = Some(val);
                    } else {
                        return Err("Missing argument for -t / --title".to_string());
                    }
                }
                "-e" | "--exec" | "--command" => {
                    let mut cmd_args: Vec<String> = Vec::new();
                    for next_arg in iter.by_ref() {
                        cmd_args.push(next_arg);
                    }
                    if let Some(cmd) = cmd_args.first() {
                        opts.command = Some(cmd.clone());
                        opts.args = cmd_args[1..].to_vec();
                    } else {
                        return Err("Missing command for -e / --exec".to_string());
                    }
                }
                "--" => {
                    let mut cmd_args: Vec<String> = Vec::new();
                    for next_arg in iter.by_ref() {
                        cmd_args.push(next_arg);
                    }
                    if let Some(cmd) = cmd_args.first() {
                        opts.command = Some(cmd.clone());
                        opts.args = cmd_args[1..].to_vec();
                    }
                }
                other if !other.starts_with('-') => {
                    let path = if other.starts_with("~/") {
                        dirs::home_dir()
                            .map(|h| h.join(&other[2..]))
                            .unwrap_or_else(|| PathBuf::from(other))
                    } else {
                        PathBuf::from(other)
                    };
                    if path.is_dir() {
                        opts.working_dir = Some(path);
                    } else {
                        opts.command = Some(other.to_string());
                    }
                }
                unknown => {
                    eprintln!("Unknown option: {}", unknown);
                }
            }
        }

        Ok(opts)
    }
}
