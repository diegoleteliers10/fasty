//! Temporary, per-terminal shell integration files and launch arguments.
//!
//! This module never edits a user's startup files. The caller should apply a
//! plan only when it launches the configured interactive shell without args.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Nushell,
}

#[derive(Clone, Debug)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Clone, Debug)]
pub struct ShellIntegrationPlan {
    pub kind: ShellKind,
    /// Arguments to add to the shell command. Preserve all caller args by
    /// skipping this plan when the shell has explicit args.
    pub args: Vec<OsString>,
    /// Environment changes required for shell startup wrappers.
    pub env: Vec<(OsString, OsString)>,
    /// Files which the caller must create before spawning the shell.
    pub files: Vec<GeneratedFile>,
}

/// Plan shell integration for a default interactive shell.
///
/// Returns `None` for explicit command args and unsupported shells. The caller
/// must materialize `files`, set `env`, and append `args` before spawning.
pub fn plan_shell_integration(
    executable: &str,
    exec_args: &[String],
    cache_dir: &Path,
    home_dir: &Path,
) -> Option<ShellIntegrationPlan> {
    if !exec_args.is_empty() {
        return None;
    }

    let name = Path::new(executable)
        .file_name()?
        .to_str()?
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    match name.as_str() {
        "bash" => Some(bash_plan(cache_dir)),
        "zsh" => Some(zsh_plan(cache_dir, home_dir)),
        "fish" => Some(fish_plan(cache_dir, home_dir)),
        "pwsh" | "powershell" => Some(powershell_plan(cache_dir)),
        "nu" => Some(nushell_plan(cache_dir, home_dir)),
        _ => None,
    }
}

fn bash_plan(cache_dir: &Path) -> ShellIntegrationPlan {
    let dir = cache_dir.join("shell_integration");
    let rc = dir.join("fastty_bashrc");
    let integration = dir.join("fastty_bash.sh");
    let contents = format!(
        "# Fastty-generated Bash startup wrapper.\n\
         __fastty_profile=\n\
         for __fastty_candidate in \"$HOME/.bash_profile\" \"$HOME/.bash_login\" \"$HOME/.profile\"; do\n\
             if [ -f \"$__fastty_candidate\" ]; then __fastty_profile=$__fastty_candidate; . \"$__fastty_profile\"; break; fi\n\
         done\n\
         if [ -f \"$HOME/.bashrc\" ] && {{ [ -z \"$__fastty_profile\" ] || ! grep -Eq '(^|[[:space:];])(\\.|source)[[:space:]].*bashrc' \"$__fastty_profile\"; }}; then . \"$HOME/.bashrc\"; fi\n\
         . {}\n",
        bash_quote(&integration)
    );
    ShellIntegrationPlan {
        kind: ShellKind::Bash,
        args: vec!["-i".into(), "--rcfile".into(), rc.as_os_str().into()],
        env: vec![],
        files: vec![
            GeneratedFile { path: rc, contents },
            GeneratedFile {
                path: integration,
                contents: BASH_INTEGRATION.into(),
            },
        ],
    }
}

fn zsh_plan(cache_dir: &Path, home: &Path) -> ShellIntegrationPlan {
    let dir = cache_dir.join("shell_integration").join("zsh");
    let original_zdotdir = std::env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf());
    let startup_names = [".zshenv", ".zprofile", ".zshrc", ".zlogin"];
    let mut files = Vec::new();
    for startup in startup_names {
        let wrapper = dir.join(startup);
        let user_file = original_zdotdir.join(startup);
        let user_source = if startup == ".zshenv" {
            format!(
                "export ZDOTDIR={}\n[ -f {} ] && source {}\n\
                 __fastty_capture_zdotdir() {{ if (( ${{+ZDOTDIR}} )); then __fastty_user_zdotdir=$ZDOTDIR; __fastty_user_zdotdir_set=1; else __fastty_user_zdotdir=$HOME; __fastty_user_zdotdir_set=0; fi }}\n\
                 __fastty_capture_zdotdir\n",
                bash_quote(&original_zdotdir),
                bash_quote(&user_file),
                bash_quote(&user_file),
            )
        } else {
            format!(
                "if [[ $__fastty_user_zdotdir_set == 1 ]]; then export ZDOTDIR=$__fastty_user_zdotdir; else unset ZDOTDIR; fi\n\
                 __fastty_user_file=\"${{__fastty_user_zdotdir}}/{startup}\"\n\
                 [ -f \"$__fastty_user_file\" ] && source \"$__fastty_user_file\"\n\
                 __fastty_capture_zdotdir\n"
            )
        };
        let integration = if startup == ".zshrc" {
            format!("source {}\n", bash_quote(&dir.join("fastty.zsh")))
        } else {
            String::new()
        };
        let finish = if startup == ".zlogin" {
            "if [[ $__fastty_user_zdotdir_set == 1 ]]; then export ZDOTDIR=$__fastty_user_zdotdir; else unset ZDOTDIR; fi\n".to_string()
        } else {
            format!("export ZDOTDIR={}\n", bash_quote(&dir))
        };
        files.push(GeneratedFile {
            path: wrapper,
            contents: format!(
                "# Fastty-generated startup wrapper.\n{user_source}{integration}{finish}"
            ),
        });
    }
    files.push(GeneratedFile {
        path: dir.join("fastty.zsh"),
        contents: ZSH_INTEGRATION.into(),
    });
    ShellIntegrationPlan {
        kind: ShellKind::Zsh,
        args: vec!["-l".into()],
        env: vec![("ZDOTDIR".into(), dir.as_os_str().into())],
        files,
    }
}

fn fish_plan(cache_dir: &Path, _home: &Path) -> ShellIntegrationPlan {
    let dir = cache_dir.join("shell_integration");
    let integration = dir.join("fastty_fish.fish");
    ShellIntegrationPlan {
        kind: ShellKind::Fish,
        // Fish runs --init-command after normal startup files.
        args: vec![
            "--login".into(),
            "--init-command".into(),
            format!("source {}", fish_quote(&integration)).into(),
        ],
        env: vec![],
        files: vec![GeneratedFile {
            path: integration,
            contents: FISH_INTEGRATION.into(),
        }],
    }
}

fn powershell_plan(cache_dir: &Path) -> ShellIntegrationPlan {
    let script = cache_dir
        .join("shell_integration")
        .join("fastty_profile.ps1");
    ShellIntegrationPlan {
        kind: ShellKind::PowerShell,
        args: vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            format!(". {}", powershell_quote(&script)).into(),
        ],
        env: vec![],
        files: vec![GeneratedFile {
            path: script,
            contents: POWERSHELL_INTEGRATION.into(),
        }],
    }
}

fn nushell_plan(cache_dir: &Path, home: &Path) -> ShellIntegrationPlan {
    let dir = cache_dir.join("shell_integration").join("nushell");
    let config = dir.join("config.nu");
    let env_config = dir.join("env.nu");
    let user_config_dir = std::env::var_os("NU_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| nushell_config_dir(home));
    let user_config = user_config_dir.join("config.nu");
    let user_env = user_config_dir.join("env.nu");
    let user_config_quoted = nu_quote(&user_config);
    let config_contents = format!(
        r#"# Fastty-generated Nushell config wrapper.
if ({0} | path exists) {{ source {0} }}
$env.config.hooks.pre_execution = ($env.config.hooks.pre_execution? | default [] | append {{|| print -n ((char escape) + "]133;B" + (char escape) + "\\") }})
$env.config.hooks.pre_prompt = ($env.config.hooks.pre_prompt? | default [] | append {{|| let code = ($env.LAST_EXIT_CODE? | default 0); print -n ((char escape) + $"]133;D;($code)" + (char escape) + "\\"); print -n ((char escape) + "]133;A" + (char escape) + "\\") }})
"#,
        user_config_quoted
    );
    let env_contents = format!(
        "# Fastty-generated Nushell environment wrapper.\nif ({} | path exists) {{ source-env {} }}\n",
        nu_quote(&user_env),
        nu_quote(&user_env)
    );
    ShellIntegrationPlan {
        kind: ShellKind::Nushell,
        args: vec![
            "--config".into(),
            config.as_os_str().into(),
            "--env-config".into(),
            env_config.as_os_str().into(),
        ],
        env: vec![],
        files: vec![
            GeneratedFile {
                path: config,
                contents: config_contents,
            },
            GeneratedFile {
                path: env_config,
                contents: env_contents,
            },
        ],
    }
}

fn nushell_config_dir(home: &Path) -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("nushell")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("nushell")
    }
}

fn bash_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn fish_quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
    )
}

fn nu_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn powershell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

const BASH_INTEGRATION: &str = r#"# Fastty OSC 133 integration.
__fastty_ready_for_command=0
__fastty_prompt() {
    local __fastty_status=$?
    printf '\033]133;D;%s\033\\' "$__fastty_status"
    printf '\033]133;A\033\\\033[?9l\033[?1000l\033[?1002l\033[?1003l\033[?1005l\033[?1006l'
}
__fastty_enable_preexec() { __fastty_ready_for_command=1; }
__fastty_debug() {
    [[ $__fastty_ready_for_command == 1 ]] || return
    [[ $BASH_COMMAND == __fastty_* ]] && return
    __fastty_ready_for_command=0
    printf '\033]133;B\033\\'
}
case ";${PROMPT_COMMAND-};" in
    *";__fastty_prompt;"*) ;;
    *) PROMPT_COMMAND="__fastty_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND};__fastty_enable_preexec" ;;
esac
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
    PS0='\[\e]133;B\e\\\]'
elif [[ -z $(trap -p DEBUG) ]]; then
    trap '__fastty_debug' DEBUG
fi
"#;

const ZSH_INTEGRATION: &str = r#"# Fastty OSC 133 integration.
autoload -Uz add-zsh-hook
__fastty_preexec() { printf '\033]133;B\033\\' }
__fastty_precmd() {
    local __fastty_status=$?
    printf '\033]133;D;%s\033\\' "$__fastty_status"
    printf '\033]133;A\033\\\033[?9l\033[?1000l\033[?1002l\033[?1003l\033[?1005l\033[?1006l'
}
add-zsh-hook -Uz preexec __fastty_preexec
add-zsh-hook -Uz precmd __fastty_precmd
"#;

const FISH_INTEGRATION: &str = r#"# Fastty OSC 133 integration.
function __fastty_preexec --on-event fish_preexec
    printf '\e]133;B\e\\'
end
function __fastty_postexec --on-event fish_postexec
    printf '\e]133;D;%s\e\\' $status
end
function __fastty_prompt --on-event fish_prompt
    printf '\e]133;A\e\\\e[?9l\e[?1000l\e[?1002l\e[?1003l\e[?1005l\e[?1006l'
end
"#;

const POWERSHELL_INTEGRATION: &str = r#"# Fastty OSC 133 integration. Dot-sourced after the user's PowerShell profiles.
$script:__fastty_original_prompt = (Get-Command prompt -CommandType Function -ErrorAction SilentlyContinue).ScriptBlock
function global:prompt {
    $status = if ($?) { 0 } else { 1 }
    if ($global:LASTEXITCODE -ne $null) { $status = $global:LASTEXITCODE }
    [Console]::Out.Write("`e]133;D;$status`e\`e]133;A`e\`e[?9l`e[?1000l`e[?1002l`e[?1003l`e[?1005l`e[?1006l")
    if ($script:__fastty_original_prompt) { & $script:__fastty_original_prompt }
    else { "PS $($executionContext.SessionState.Path.CurrentLocation)> " }
}
if (Get-Module -Name PSReadLine) {
    $options = Get-PSReadLineOption
    $property = $options.PSObject.Properties['CommandValidationHandler']
    if ($null -ne $property) {
        $script:__fastty_original_validation = $property.Value
        Set-PSReadLineOption -CommandValidationHandler {
            param($line)
            if ($script:__fastty_original_validation) {
                $line = & $script:__fastty_original_validation $line
            }
            $global:LASTEXITCODE = $null
            [Console]::Out.Write("`e]133;B`e\")
            return $line
        }
    }
}
"#;
