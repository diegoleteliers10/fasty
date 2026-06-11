//! Git status types and worktree helpers shared between main and renderer.

use std::path::{Path, PathBuf};

/// Cached git status for a single tab. Populated by `detect_git_status` in main.rs.
#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    pub last_commit_summary: String,
    pub checked_at: std::time::Instant,
}

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.modified == 0 && self.staged == 0 && self.untracked == 0
    }
}

/// A single entry from `git worktree list --porcelain`.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub commit: String,
}

impl Worktree {
    pub fn short_commit(&self) -> &str {
        if self.commit.len() >= 7 {
            &self.commit[..7]
        } else {
            &self.commit
        }
    }

    pub fn short_branch(&self) -> &str {
        match &self.branch {
            Some(b) => b.rsplit('/').next().unwrap_or(b.as_str()),
            None => "(detached)",
        }
    }
}

/// Parse the porcelain output of `git worktree list --porcelain`.
pub fn parse_worktree_list(output: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(Worktree {
                path: PathBuf::from(rest),
                branch: None,
                commit: String::new(),
            });
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            if let Some(c) = current.as_mut() {
                c.commit = rest.to_string();
            }
        } else if let Some(rest) = line.strip_prefix("branch ") {
            if let Some(c) = current.as_mut() {
                c.branch = Some(rest.to_string());
            }
        }
    }
    if let Some(prev) = current {
        out.push(prev);
    }
    out
}

fn run_git(cwd: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn is_git_repo(cwd: &Path) -> bool {
    run_git(Some(cwd), &["rev-parse", "--show-toplevel"]).is_some()
}

pub fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let s = run_git(Some(cwd), &["rev-parse", "--show-toplevel"])?;
    let trimmed = s.trim();
    Some(PathBuf::from(trimmed))
}

pub fn list_worktrees(cwd: &Path) -> Vec<Worktree> {
    match run_git(Some(cwd), &["worktree", "list", "--porcelain"]) {
        Some(s) => parse_worktree_list(&s),
        None => Vec::new(),
    }
}

/// Sanitize a branch name for use as a directory suffix: replace `/` with `-`,
/// drop characters that are awkward in paths.
pub fn sanitize_branch_for_path(branch: &str) -> String {
    branch
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '-' | '_' | '.'))
        .collect()
}

/// Run `git worktree add <path> -b <branch>` and return the new worktree path on success.
pub fn create_worktree(toplevel: &Path, new_branch: &str) -> Option<PathBuf> {
    let parent = toplevel.parent()?;
    let dir_name = toplevel.file_name()?.to_string_lossy().into_owned();
    let new_path = parent.join(format!("{}-{}", dir_name, sanitize_branch_for_path(new_branch)));
    let new_path_str = new_path.to_string_lossy().into_owned();
    run_git(
        Some(toplevel),
        &["worktree", "add", &new_path_str, "-b", new_branch],
    )?;
    Some(new_path)
}
