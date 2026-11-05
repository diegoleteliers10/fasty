//! Tab identity: the official mark of the program that owns the tab, plus the
//! activity indicator.
//!
//! A tab shows its brand mark when the foreground program is a known agent or
//! tool, a green terminal when it is a shell, and the deck icon for other
//! programs. The three-dot loader appears only while the program reports
//! `working` through `OSC 7501`.

use std::time::Duration;

use gpui::*;

use super::brand_assets as art;
use super::theme::Theme;
use crate::program_status::ProgramState;

/// Every program with a bundled mark. `brand_assets` holds the artwork.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrandMark {
    // Agents and agent runtimes.
    Claude,
    OpenCode,
    Codex,
    Gemini,
    Copilot,
    Cursor,
    Amp,
    Qwen,
    Cline,
    Windsurf,
    Mistral,
    Kimi,
    LmStudio,
    Pi,
    Antigravity,
    Hermes,
    // Languages, runtimes, and tools.
    Node,
    Bun,
    Deno,
    Npm,
    Pnpm,
    Yarn,
    Python,
    Rust,
    Go,
    Ruby,
    Php,
    Java,
    Kotlin,
    Swift,
    Dotnet,
    Lua,
    Elixir,
    Zig,
    Docker,
    Terraform,
    Kubernetes,
    Git,
    Neovim,
    Vim,
    Tmux,
    Redis,
    Sqlite,
    Postgresql,
    Mongodb,
    Cmake,
    Gradle,
    Ollama,
    HuggingFace,
    Haskell,
    Clojure,
    R,
    Dart,
    Flutter,
}

impl BrandMark {
    /// Every bundled mark, for the artwork test and for menus.
    pub const ALL: &'static [BrandMark] = &[
        BrandMark::Claude,
        BrandMark::OpenCode,
        BrandMark::Codex,
        BrandMark::Gemini,
        BrandMark::Copilot,
        BrandMark::Cursor,
        BrandMark::Amp,
        BrandMark::Qwen,
        BrandMark::Cline,
        BrandMark::Windsurf,
        BrandMark::Mistral,
        BrandMark::Kimi,
        BrandMark::LmStudio,
        BrandMark::Pi,
        BrandMark::Antigravity,
        BrandMark::Hermes,
        BrandMark::Node,
        BrandMark::Bun,
        BrandMark::Deno,
        BrandMark::Npm,
        BrandMark::Pnpm,
        BrandMark::Yarn,
        BrandMark::Python,
        BrandMark::Rust,
        BrandMark::Go,
        BrandMark::Ruby,
        BrandMark::Php,
        BrandMark::Java,
        BrandMark::Kotlin,
        BrandMark::Swift,
        BrandMark::Dotnet,
        BrandMark::Lua,
        BrandMark::Elixir,
        BrandMark::Zig,
        BrandMark::Docker,
        BrandMark::Terraform,
        BrandMark::Kubernetes,
        BrandMark::Git,
        BrandMark::Neovim,
        BrandMark::Vim,
        BrandMark::Tmux,
        BrandMark::Redis,
        BrandMark::Sqlite,
        BrandMark::Postgresql,
        BrandMark::Mongodb,
        BrandMark::Cmake,
        BrandMark::Gradle,
        BrandMark::Ollama,
        BrandMark::HuggingFace,
        BrandMark::Haskell,
        BrandMark::Clojure,
        BrandMark::R,
        BrandMark::Dart,
        BrandMark::Flutter,
    ];
}

/// What a tab draws on its left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabGlyph {
    Brand(BrandMark),
    /// A program that is an agent but has no bundled mark.
    Agent,
    /// A non-agent program with a deck icon. Never carries `Terminal`: a
    /// program with no icon of its own keeps [`TabGlyph::Shell`].
    Deck(icons::common::IconType),
    /// The shell, drawn as the usual green terminal.
    Shell,
}

fn artwork(mark: BrandMark) -> Option<&'static art::BrandArtwork> {
    Some(match mark {
        BrandMark::Claude => &art::CLAUDE,
        BrandMark::OpenCode => &art::OPENCODE,
        BrandMark::Codex => &art::OPENAI,
        BrandMark::Gemini => &art::GEMINI,
        BrandMark::Copilot => &art::COPILOT,
        BrandMark::Cursor => &art::CURSOR,
        BrandMark::Amp => &art::AMP,
        BrandMark::Qwen => &art::QWEN,
        BrandMark::Cline => &art::CLINE,
        BrandMark::Windsurf => &art::WINDSURF,
        BrandMark::Mistral => &art::MISTRAL,
        BrandMark::Kimi => &art::KIMI,
        BrandMark::LmStudio => &art::LM_STUDIO,
        BrandMark::Pi => &art::PI,
        BrandMark::Antigravity => &art::ANTIGRAVITY,
        // Hermes ships as a raster asset, not an SVG document.
        BrandMark::Hermes => return None,
        BrandMark::Node => &art::NODE,
        BrandMark::Bun => &art::BUN,
        BrandMark::Deno => &art::DENO,
        BrandMark::Npm => &art::NPM,
        BrandMark::Pnpm => &art::PNPM,
        BrandMark::Yarn => &art::YARN,
        BrandMark::Python => &art::PYTHON,
        BrandMark::Rust => &art::RUST,
        BrandMark::Go => &art::GO,
        BrandMark::Ruby => &art::RUBY,
        BrandMark::Php => &art::PHP,
        BrandMark::Java => &art::JAVA,
        BrandMark::Kotlin => &art::KOTLIN,
        BrandMark::Swift => &art::SWIFT,
        BrandMark::Dotnet => &art::DOTNET,
        BrandMark::Lua => &art::LUA,
        BrandMark::Elixir => &art::ELIXIR,
        BrandMark::Zig => &art::ZIG,
        BrandMark::Docker => &art::DOCKER,
        BrandMark::Terraform => &art::TERRAFORM,
        BrandMark::Kubernetes => &art::KUBERNETES,
        BrandMark::Git => &art::GIT,
        BrandMark::Neovim => &art::NEOVIM,
        BrandMark::Vim => &art::VIM,
        BrandMark::Tmux => &art::TMUX,
        BrandMark::Redis => &art::REDIS,
        BrandMark::Sqlite => &art::SQLITE,
        BrandMark::Postgresql => &art::POSTGRESQL,
        BrandMark::Mongodb => &art::MONGODB,
        BrandMark::Cmake => &art::CMAKE,
        BrandMark::Gradle => &art::GRADLE,
        BrandMark::Ollama => &art::OLLAMA,
        BrandMark::HuggingFace => &art::HUGGING_FACE,
        BrandMark::Haskell => &art::HASKELL,
        BrandMark::Clojure => &art::CLOJURE,
        BrandMark::R => &art::R_LANG,
        BrandMark::Dart => &art::DART,
        BrandMark::Flutter => &art::FLUTTER,
    })
}

fn brand_svg_bytes(mark: BrandMark) -> Option<&'static [u8]> {
    // One leak per mark, once per process: the renderer needs `'static` bytes.
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<BrandMark, &'static [u8]>>,
    > = std::sync::OnceLock::new();
    let artwork = artwork(mark)?;
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(bytes) = cache.get(&mark) {
        return Some(bytes);
    }
    let leaked: &'static [u8] = Box::leak(artwork.svg.as_bytes().to_vec().into_boxed_slice());
    cache.insert(mark, leaked);
    Some(leaked)
}

fn hermes_logo_image() -> Option<std::sync::Arc<RenderImage>> {
    static LOGO: std::sync::OnceLock<Option<std::sync::Arc<RenderImage>>> =
        std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        let bytes = include_bytes!("../../assets/brands/hermes.png");
        let decoded = image::load_from_memory(bytes).ok()?;
        let mut data = decoded.into_rgba8();
        // gpui expects BGRA.
        for pixel in data.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let frame = image::Frame::new(data);
        Some(std::sync::Arc::new(RenderImage::new(smallvec::smallvec![
            frame
        ])))
    })
    .clone()
}

/// Split an app name or process name into lowercase tokens, so
/// `claude-code`, `/opt/homebrew/bin/opencode` and `Claude Code` all resolve.
fn tokens(raw: &str) -> Vec<String> {
    let lower = raw.to_lowercase();
    let base = std::path::Path::new(&lower)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&lower);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    base.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn brand_from_token(token: &str) -> Option<BrandMark> {
    let mark = match token {
        // Agents.
        "claude" | "anthropic" => BrandMark::Claude,
        "opencode" => BrandMark::OpenCode,
        "codex" | "openai" | "chatgpt" => BrandMark::Codex,
        "gemini" => BrandMark::Gemini,
        "copilot" => BrandMark::Copilot,
        "cursor" => BrandMark::Cursor,
        "amp" => BrandMark::Amp,
        "qwen" => BrandMark::Qwen,
        "cline" => BrandMark::Cline,
        "windsurf" | "codeium" => BrandMark::Windsurf,
        "mistral" | "codestral" => BrandMark::Mistral,
        "kimi" | "moonshot" => BrandMark::Kimi,
        "lmstudio" | "lms" => BrandMark::LmStudio,
        "pi" => BrandMark::Pi,
        "agy" | "antigravity" => BrandMark::Antigravity,
        "hermes" => BrandMark::Hermes,
        // Languages and runtimes.
        "node" | "nodejs" | "tsc" | "vite" | "next" | "jest" | "tsx" | "tsnode" => BrandMark::Node,
        "bun" | "bunx" => BrandMark::Bun,
        "deno" => BrandMark::Deno,
        "npm" | "npx" => BrandMark::Npm,
        "pnpm" => BrandMark::Pnpm,
        "yarn" => BrandMark::Yarn,
        "python" | "python3" | "py" | "ipython" | "pip" | "pip3" | "poetry" | "uv" | "uvx"
        | "pytest" | "jupyter" | "flask" => BrandMark::Python,
        "cargo" | "rustc" | "rustup" | "rustfmt" | "clippy" => BrandMark::Rust,
        "go" | "gofmt" | "goimports" | "air" => BrandMark::Go,
        "ruby" | "irb" | "rails" | "bundle" | "gem" | "rake" => BrandMark::Ruby,
        "php" | "composer" | "phpunit" | "artisan" => BrandMark::Php,
        "java" | "javac" | "mvn" | "sbt" | "jdk" => BrandMark::Java,
        "kotlin" | "kotlinc" => BrandMark::Kotlin,
        "swift" | "xcodebuild" => BrandMark::Swift,
        "dotnet" | "csc" | "msbuild" => BrandMark::Dotnet,
        "lua" | "luajit" | "love" => BrandMark::Lua,
        "elixir" | "mix" | "iex" => BrandMark::Elixir,
        "zig" => BrandMark::Zig,
        "dart" => BrandMark::Dart,
        "flutter" => BrandMark::Flutter,
        "haskell" | "ghc" | "cabal" | "stack" => BrandMark::Haskell,
        "clojure" | "clj" | "lein" => BrandMark::Clojure,
        "rscript" | "radian" => BrandMark::R,
        // Infrastructure and tools.
        "docker" | "compose" | "podman" => BrandMark::Docker,
        "terraform" | "tofu" | "terragrunt" => BrandMark::Terraform,
        "kubectl" | "k9s" | "helm" | "minikube" | "k3s" => BrandMark::Kubernetes,
        "git" | "gh" | "lazygit" | "gitui" => BrandMark::Git,
        "nvim" => BrandMark::Neovim,
        "vim" | "vi" => BrandMark::Vim,
        "tmux" => BrandMark::Tmux,
        "redis" => BrandMark::Redis,
        "sqlite" | "sqlite3" => BrandMark::Sqlite,
        "psql" | "postgres" | "pgctl" => BrandMark::Postgresql,
        "mongo" | "mongosh" | "mongod" => BrandMark::Mongodb,
        "cmake" => BrandMark::Cmake,
        "gradle" | "gradlew" => BrandMark::Gradle,
        "ollama" => BrandMark::Ollama,
        "hf" | "huggingface" => BrandMark::HuggingFace,
        _ => return None,
    };
    Some(mark)
}

fn is_agent_token(token: &str) -> bool {
    matches!(
        token,
        "aider" | "goose" | "crush" | "droid" | "factory" | "codebuff"
    )
}

fn is_shell(token: &str) -> bool {
    matches!(
        token,
        "zsh"
            | "bash"
            | "fish"
            | "sh"
            | "nu"
            | "nushell"
            | "pwsh"
            | "tcsh"
            | "dash"
            | "ksh"
            | "xonsh"
            | "elvish"
    )
}

/// A program without a brand mark keeps its deck icon. A program that has no
/// icon of its own keeps the shell mark, so one prompt never shows two
/// terminal marks.
fn deck_glyph(icon: icons::common::IconType) -> TabGlyph {
    if icon == icons::common::IconType::Terminal {
        TabGlyph::Shell
    } else {
        TabGlyph::Deck(icon)
    }
}

fn fallback_icon(token: &str) -> icons::common::IconType {
    match token {
        "ssh" | "mysql" | "mysqld" | "mariadb" | "gcloud" | "gsutil" | "az" | "aws" | "doctl" => {
            icons::common::IconType::Server
        }
        "make" | "ninja" | "meson" | "bazel" => icons::common::IconType::Hammer,
        "htop" | "btop" | "top" => icons::common::IconType::Cpu,
        _ => icons::common::IconType::Terminal,
    }
}

/// Pick the tab glyph from the foreground process and the `OSC 7501` app name.
///
/// A shell prompt wins over an app record: an agent that finished keeps its
/// `done` record, and the pane is a shell again once it exits.
pub fn tab_glyph(app: Option<&str>, process: Option<&str>) -> TabGlyph {
    let process_tokens = process.map(tokens);
    if process_tokens
        .as_ref()
        .is_some_and(|tokens| tokens.iter().any(|token| is_shell(token)))
    {
        return TabGlyph::Shell;
    }
    if app.is_some_and(|name| name.to_lowercase().contains("lm studio")) {
        return TabGlyph::Brand(BrandMark::LmStudio);
    }
    let app_tokens = app.map(tokens);
    let sources: [&[String]; 2] = [
        app_tokens.as_deref().unwrap_or(&[]),
        process_tokens.as_deref().unwrap_or(&[]),
    ];
    for tokens in sources {
        for token in tokens {
            if let Some(mark) = brand_from_token(token) {
                return TabGlyph::Brand(mark);
            }
            if is_agent_token(token) {
                return TabGlyph::Agent;
            }
        }
    }
    if let Some(token) = process_tokens.as_ref().and_then(|tokens| tokens.first()) {
        return deck_glyph(fallback_icon(token));
    }
    match process {
        Some(name) => {
            let (icon, _) = super::icons::get_deck_process_icon(name);
            deck_glyph(icon)
        }
        None => TabGlyph::Shell,
    }
}

/// The loader shows only while the program reports `working`.
pub fn is_working(state: Option<ProgramState>) -> bool {
    state == Some(ProgramState::Working)
}

fn hex_is_dark(hex: u32) -> bool {
    let red = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let green = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let blue = (hex & 0xFF) as f32 / 255.0;
    0.2126 * red + 0.7152 * green + 0.0722 * blue < 0.3
}

pub fn brand_color(mark: BrandMark, theme: Theme) -> Hsla {
    match artwork(mark).and_then(|artwork| artwork.color) {
        // A near-black brand mark disappears on a dark card, so it uses the
        // text color instead.
        Some(hex) if !hex_is_dark(hex) => crate::ui::theme::rgb_to_hsla(
            ((hex >> 16) & 0xFF) as u8,
            ((hex >> 8) & 0xFF) as u8,
            (hex & 0xFF) as u8,
        ),
        _ => theme.foreground,
    }
}

/// Draw the glyph at a fixed size. Inactive tabs dim the whole mark so a
/// multi-color brand keeps its shape.
pub fn render_tab_glyph(glyph: TabGlyph, theme: Theme, active: bool, size: f32) -> AnyElement {
    let dim = if active { 1.0 } else { 0.68 };
    match glyph {
        TabGlyph::Brand(mark) => {
            if mark == BrandMark::Hermes {
                return match hermes_logo_image() {
                    Some(logo) => img(logo)
                        .w(px(size))
                        .h(px(size))
                        .flex_shrink_0()
                        .opacity(dim)
                        .into_any_element(),
                    None => {
                        super::icons::render_icon(icons::common::IconType::Bot, theme.accent, size)
                            .into_any_element()
                    }
                };
            }
            let Some(bytes) = brand_svg_bytes(mark) else {
                return super::icons::render_icon(icons::common::IconType::Bot, theme.accent, size)
                    .into_any_element();
            };
            svg()
                .data(bytes)
                .text_color(brand_color(mark, theme))
                .w(px(size))
                .h(px(size))
                .flex_shrink_0()
                .opacity(dim)
                .into_any_element()
        }
        TabGlyph::Agent => super::icons::render_icon(
            icons::common::IconType::Bot,
            theme.accent.opacity(dim),
            size,
        )
        .into_any_element(),
        TabGlyph::Deck(icon) => {
            let color = if active {
                theme.accent
            } else {
                theme.muted_strong
            };
            super::icons::render_icon(icon, color, size).into_any_element()
        }
        TabGlyph::Shell => super::icons::render_icon(
            icons::common::IconType::SquareTerminal,
            theme.green.opacity(dim),
            size,
        )
        .into_any_element(),
    }
}

/// The three-dot activity loader. One crest travels across the dots and each
/// dot fades in and out of it, so the row loops without a seam.
pub fn render_working_dots(
    color: Hsla,
    dot_size: f32,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    /// One pass of the crest, slow enough to read as breathing.
    const CYCLE: Duration = Duration::from_millis(1_200);
    /// Share of the cycle each dot trails the dot before it.
    const LAG: f32 = 0.13;
    /// Faintest a dot gets, so the row keeps its shape between passes.
    const REST: f32 = 0.22;

    let gap = dot_size * 0.55;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(gap))
        .flex_shrink_0()
        .ml(px(4.))
        .with_animation(
            id,
            Animation::new(CYCLE).repeat_synced().with_max_fps(30.0),
            move |element, delta| {
            element.children((0..3usize).map(move |index| {
                let phase = delta - index as f32 * LAG;
                let crest = 0.5 + 0.5 * (phase.rem_euclid(1.0) * std::f32::consts::TAU).cos();
                div()
                    .w(px(dot_size))
                    .h(px(dot_size))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(color.opacity(REST + (1.0 - REST) * crest * crest))
            }))
        })
}

#[cfg(test)]
mod tests {
    // Explicit imports: a glob of `super::*` would also pull gpui's `test`
    // attribute macro into scope and shadow the built-in test attribute.
    use super::{brand_svg_bytes, deck_glyph, is_working, tab_glyph, BrandMark, TabGlyph};
    use crate::program_status::ProgramState;

    #[test]
    fn an_agent_app_beats_its_runtime_process() {
        assert_eq!(
            tab_glyph(Some("claude"), Some("node")),
            TabGlyph::Brand(BrandMark::Claude)
        );
        assert_eq!(
            tab_glyph(Some("opencode"), Some("node")),
            TabGlyph::Brand(BrandMark::OpenCode)
        );
        assert_eq!(
            tab_glyph(None, Some("/opt/homebrew/bin/codex")),
            TabGlyph::Brand(BrandMark::Codex)
        );
    }

    #[test]
    fn a_shell_prompt_wins_over_a_finished_app_record() {
        assert_eq!(tab_glyph(Some("claude"), Some("zsh")), TabGlyph::Shell);
        assert_eq!(tab_glyph(Some("opencode"), Some("bash")), TabGlyph::Shell);
    }

    #[test]
    fn a_plain_shell_reads_as_a_shell() {
        assert_eq!(tab_glyph(None, Some("zsh")), TabGlyph::Shell);
        assert_eq!(tab_glyph(Some("zsh"), Some("zsh")), TabGlyph::Shell);
        assert_eq!(tab_glyph(None, Some("bash")), TabGlyph::Shell);
        assert_eq!(tab_glyph(None, Some("dash")), TabGlyph::Shell);
        assert_eq!(tab_glyph(None, Some("ksh")), TabGlyph::Shell);
        assert_eq!(tab_glyph(None, Some("xonsh")), TabGlyph::Shell);
        assert_eq!(tab_glyph(None, Some("elvish")), TabGlyph::Shell);
    }

    #[test]
    fn known_programs_use_their_brand_mark() {
        assert_eq!(
            tab_glyph(None, Some("cargo")),
            TabGlyph::Brand(BrandMark::Rust)
        );
        assert_eq!(
            tab_glyph(None, Some("node")),
            TabGlyph::Brand(BrandMark::Node)
        );
        assert_eq!(
            tab_glyph(None, Some("python3.12")),
            TabGlyph::Brand(BrandMark::Python)
        );
        assert_eq!(
            tab_glyph(None, Some("nvim")),
            TabGlyph::Brand(BrandMark::Neovim)
        );
    }

    #[test]
    fn new_agents_are_mapped() {
        assert_eq!(
            tab_glyph(Some("pi"), Some("node")),
            TabGlyph::Brand(BrandMark::Pi)
        );
        assert_eq!(
            tab_glyph(Some("agy"), Some("node")),
            TabGlyph::Brand(BrandMark::Antigravity)
        );
        assert_eq!(
            tab_glyph(Some("hermes"), Some("node")),
            TabGlyph::Brand(BrandMark::Hermes)
        );
        assert_eq!(
            tab_glyph(Some("antigravity-acp"), Some("node")),
            TabGlyph::Brand(BrandMark::Antigravity)
        );
        assert_eq!(
            tab_glyph(Some("opencode-v2"), Some("node")),
            TabGlyph::Brand(BrandMark::OpenCode)
        );
    }

    #[test]
    fn lm_studio_matches_its_app_name_and_cli() {
        assert_eq!(
            tab_glyph(Some("LM Studio"), None),
            TabGlyph::Brand(BrandMark::LmStudio)
        );
        assert_eq!(
            tab_glyph(None, Some("lms")),
            TabGlyph::Brand(BrandMark::LmStudio)
        );
        assert_eq!(tab_glyph(Some("LM Studio"), Some("zsh")), TabGlyph::Shell);
    }

    #[test]
    fn an_unknown_agent_still_reads_as_an_agent() {
        assert_eq!(tab_glyph(Some("aider"), Some("python3")), TabGlyph::Agent);
        assert_eq!(tab_glyph(Some("goose"), None), TabGlyph::Agent);
    }

    #[test]
    fn unknown_programs_fall_back_to_the_shell_mark() {
        assert_eq!(
            tab_glyph(None, Some("htop")),
            TabGlyph::Deck(icons::common::IconType::Cpu)
        );
        // A program with no icon of its own keeps the shell mark rather than a
        // second, dimmer terminal mark.
        assert_eq!(tab_glyph(None, Some("unknown-tool")), TabGlyph::Shell);
        assert_eq!(
            tab_glyph(None, Some("ssh")),
            TabGlyph::Deck(icons::common::IconType::Server)
        );
    }

    #[test]
    fn a_bare_terminal_icon_becomes_the_shell_mark() {
        assert_eq!(
            deck_glyph(icons::common::IconType::Terminal),
            TabGlyph::Shell
        );
        assert_eq!(
            deck_glyph(icons::common::IconType::Cpu),
            TabGlyph::Deck(icons::common::IconType::Cpu)
        );
    }

    #[test]
    fn names_with_prefixes_and_separators_resolve() {
        assert_eq!(
            tab_glyph(Some("Claude Code"), None),
            TabGlyph::Brand(BrandMark::Claude)
        );
        assert_eq!(
            tab_glyph(None, Some("/usr/local/bin/opencode.exe")),
            TabGlyph::Brand(BrandMark::OpenCode)
        );
        assert_eq!(
            tab_glyph(None, Some("/opt/homebrew/bin/python3.12")),
            TabGlyph::Brand(BrandMark::Python)
        );
    }

    #[test]
    fn only_working_shows_the_loader() {
        assert!(is_working(Some(ProgramState::Working)));
        assert!(!is_working(Some(ProgramState::Done)));
        assert!(!is_working(Some(ProgramState::Blocked)));
        assert!(!is_working(Some(ProgramState::Error)));
        assert!(!is_working(None));
    }

    #[test]
    fn every_brand_mark_has_artwork() {
        for mark in BrandMark::ALL {
            if *mark == BrandMark::Hermes {
                continue;
            }
            let svg = brand_svg_bytes(*mark).expect("svg artwork");
            assert!(svg.starts_with(b"<svg"));
            assert!(svg.windows(5).any(|window| window == b"<path"));
        }
    }
}
