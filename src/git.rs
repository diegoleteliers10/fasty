//! Git status types and worktree helpers shared between main and renderer.

use std::path::{Path, PathBuf};
use crate::event_listener::EventSender;
use crate::terminal_state::AppEvent;
use notify::Watcher;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SyncStatus {
    #[default]
    UpToDate,     // ✓
    Behind(usize), // needs pull
    Ahead(usize),  // needs push
    Diverged,      // needs merge/rebase
    Unknown,       // no remote
}

/// Cached git status for a single tab. Populated by `detect_git_status` in main.rs.
#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,           // "main"
    pub is_detached: bool,        // true when HEAD is a raw commit, not a branch ref
    pub ahead: usize,             // commits ahead of remote
    pub behind: usize,            // commits behind remote
    pub staged: usize,            // staged files count
    pub unstaged: usize,          // modified unstaged count
    pub untracked: usize,         // untracked files count (? count)
    pub sync_status: SyncStatus,
    pub last_commit_hash: String, // first 7 chars
    pub last_commit_summary: String, // hash + commit subject line
    pub remote_url: Option<String>, // origin URL (e.g. https://github.com/user/repo)

}

pub type GitInfo = GitStatus;

impl GitStatus {

}

impl Default for GitStatus {
    fn default() -> Self {
        Self {
            branch: String::new(),
            is_detached: false,
            ahead: 0,
            behind: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            sync_status: SyncStatus::default(),
            last_commit_hash: String::new(),
            last_commit_summary: String::new(),
            remote_url: None,
        }
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
    git2::Repository::discover(cwd).is_ok()
}

pub fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let repo = git2::Repository::discover(cwd).ok()?;
    let path = repo.workdir()?;
    Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
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

pub fn fetch_git_info(repo_path: &Path) -> Option<GitInfo> {
    let repo = git2::Repository::discover(repo_path).ok()?;

    // Branch name + detached HEAD detection
    let head = repo.head().ok()?;
    let is_detached = !head.is_branch();
    let branch = head.shorthand().unwrap_or("HEAD").to_string();

    // Last commit
    let commit = head.peel_to_commit().ok()?;
    let hash = commit.id().to_string();
    let hash_short = if hash.len() >= 7 { hash[..7].to_string() } else { hash.clone() };
    let summary = commit.summary().unwrap_or("").to_string();
    
    // Ahead/behind vs remote
    let (ahead, behind) = get_ahead_behind(&repo, &branch);
    
    // File status counts
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    
    let mut staged = 0usize;
    let mut unstaged = 0usize;
    let mut untracked = 0usize;
    
    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW | git2::Status::INDEX_MODIFIED | 
            git2::Status::INDEX_DELETED | git2::Status::INDEX_RENAMED
        ) { staged += 1; }
        if s.intersects(
            git2::Status::WT_MODIFIED | git2::Status::WT_DELETED
        ) { unstaged += 1; }
        if s.contains(git2::Status::WT_NEW) { untracked += 1; }
    }
    
    let sync_status = {
        let local_refname = format!("refs/heads/{}", branch);
        let remote_refname = format!("refs/remotes/origin/{}", branch);
        let has_local = repo.refname_to_id(&local_refname).is_ok();
        let has_remote = repo.refname_to_id(&remote_refname).is_ok();
        if !has_local || !has_remote {
            SyncStatus::Unknown
        } else {
            match (ahead, behind) {
                (0, 0) => SyncStatus::UpToDate,
                (a, 0) => SyncStatus::Ahead(a),
                (0, b) => SyncStatus::Behind(b),
                _      => SyncStatus::Diverged,
            }
        }
    };

    let remote_url = repo.find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(|s| s.to_string()));

    Some(GitInfo {
        branch,
        is_detached,
        ahead,
        behind,
        staged,
        unstaged,
        untracked,
        sync_status,
        last_commit_hash: hash_short,
        last_commit_summary: format!("{} {}", &hash[..hash.len().min(7)], summary),
        remote_url,
    })
}

fn get_ahead_behind(repo: &git2::Repository, branch: &str) -> (usize, usize) {
    let local = match repo.refname_to_id(&format!("refs/heads/{}", branch)) {
        Ok(id) => id,
        Err(_) => return (0, 0),
    };
    let remote = match repo.refname_to_id(&format!("refs/remotes/origin/{}", branch)) {
        Ok(id) => id,
        Err(_) => return (0, 0),
    };
    repo.graph_ahead_behind(local, remote).unwrap_or((0, 0))
}



pub struct GitWatcherManager {
    watchers: std::collections::HashMap<PathBuf, notify::RecommendedWatcher>,
    sender: EventSender,
}

impl GitWatcherManager {
    pub fn new(sender: EventSender) -> Self {
        Self {
            watchers: std::collections::HashMap::new(),
            sender,
        }
    }

    pub fn watch_repo(&mut self, repo_path: &Path) {
        let repo_path = repo_path.to_path_buf();
        if self.watchers.contains_key(&repo_path) {
            return;
        }

        let git_dir = repo_path.join(".git");
        if !git_dir.exists() {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(_) => return,
        };

        for path in &["HEAD", "index", "COMMIT_EDITMSG", "refs/heads"] {
            let _ = watcher.watch(
                &git_dir.join(path),
                notify::RecursiveMode::NonRecursive,
            );
        }

        let sender_clone = self.sender.clone();
        let repo_path_clone = repo_path.clone();

        std::thread::spawn(move || {
            loop {
                if rx.recv().is_ok() {
                    // Debounce: drain rapid successive events
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    while rx.try_recv().is_ok() {}

                    sender_clone.send(AppEvent::GitRepoChanged {
                        repo_path: repo_path_clone.clone(),
                    });
                } else {
                    break;
                }
            }
        });

        self.watchers.insert(repo_path, watcher);
    }

    pub fn prune_unreferenced(&mut self, active_repos: &std::collections::HashSet<PathBuf>) {
        self.watchers.retain(|path, _| active_repos.contains(path));
    }
}
