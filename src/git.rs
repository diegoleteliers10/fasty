//! Git status types and worktree helpers shared between main and renderer.

use std::path::{Path, PathBuf};
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
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
#[derive(Default)]
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
    pub project_panel: GitPanelSnapshot,

}

#[derive(Debug, Clone, Default)]
pub struct GitPanelSnapshot {
    pub availability: GitPanelAvailability,
    pub changes: Vec<GitFileChange>,
    pub commits: Vec<GitCommit>,
    pub history_has_more: bool,
    pub untracked_file_count: usize,
    pub changes_truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GitPanelAvailability {
    #[default]
    Loading,
    Ready,
    NoProject,
    NoRepository,
    FetchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileChange {
    pub path: String,
    pub index: Option<GitFileSide>,
    pub worktree: Option<GitFileSide>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileSide {
    pub status: GitFileStatus,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFileStatus {
    Added,
    Modified,
    Untracked,
    Deleted,
    Renamed,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectGitAction {
    StagePath(String),
    UnstagePath(String),
    StageAll,
    UnstageAll,
    RestoreAllChanges,
    StashAll,
    StashTracked,
    StashPop,
    ViewStash,
    DiscardTrackedChanges,
    TrashUntrackedFiles,
}

impl ProjectGitAction {
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Self::RestoreAllChanges | Self::StashPop | Self::DiscardTrackedChanges | Self::TrashUntrackedFiles)
    }

    pub fn moves_to_os_trash(&self) -> bool {
        matches!(self, Self::TrashUntrackedFiles)
    }

    pub fn confirmation_message(&self) -> Option<&'static str> {
        match self {
            Self::RestoreAllChanges => Some("Restore tracked files and remove staged changes."),
            Self::StashPop => Some("Apply and remove the latest stash."),
            Self::DiscardTrackedChanges => Some("Discard unstaged tracked file changes permanently."),
            Self::TrashUntrackedFiles => Some("Move untracked files to your system Trash."),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::StagePath(_) => "Stage file",
            Self::UnstagePath(_) => "Unstage file",
            Self::StageAll => "Stage All",
            Self::UnstageAll => "Unstage All",
            Self::RestoreAllChanges => "Restore All Changes",
            Self::StashAll => "Stash All",
            Self::StashTracked => "Stash Tracked",
            Self::StashPop => "Stash Pop",
            Self::ViewStash => "View Stash",
            Self::DiscardTrackedChanges => "Discard Tracked Changes",
            Self::TrashUntrackedFiles => "Trash Untracked Files",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGitConfirmation {
    pub action: ProjectGitAction,
    pub tab_id: usize,
    pub cwd: PathBuf,
}

impl ProjectGitConfirmation {
    pub fn matches(&self, active_tab_id: usize, active_cwd: Option<&Path>) -> bool {
        self.tab_id == active_tab_id && active_cwd == Some(self.cwd.as_path())
    }
}

pub fn project_git_repository_root(cwd: &Path) -> Result<PathBuf, String> {
    let repo = git2::Repository::discover(cwd).map_err(|error| format!("Could not find Git repository: {error}"))?;
    let root = repo.workdir().ok_or_else(|| "Git repository has no worktree".to_string())?;
    root.canonicalize().map_err(|error| format!("Could not resolve Git repository root: {error}"))
}

/// Run one project-panel action with argument boundaries preserved.
/// Destructive actions require an explicit UI confirmation before this call.
pub fn run_project_git_action(cwd: &Path, action: &ProjectGitAction) -> Result<String, String> {
    use ProjectGitAction as Action;
    let repo_root = project_git_repository_root(cwd)?;
    if action.moves_to_os_trash() {
        return trash_untracked_files(&repo_root);
    }
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(&repo_root);
    match action {
        Action::StagePath(path) => { command.args(["add", "--"]).arg(path); }
        Action::UnstagePath(path) => { command.args(["restore", "--staged", "--"]).arg(path); }
        Action::StageAll => { command.args(["add", "-A", "--", "."]); }
        Action::UnstageAll => { command.args(["reset", "HEAD", "--", "."]); }
        Action::RestoreAllChanges => { command.args(["restore", "--source=HEAD", "--staged", "--worktree", "--", "."]); }
        Action::StashAll => { command.args(["stash", "push", "--include-untracked", "-m", "Fastty project panel"]); }
        Action::StashTracked => { command.args(["stash", "push", "-m", "Fastty project panel", "--", "."]); }
        Action::StashPop => { command.args(["stash", "pop"]); }
        Action::ViewStash => { command.args(["stash", "list", "--max-count=20", "--format=%gd %s (%cr)"]); }
        Action::DiscardTrackedChanges => { command.args(["restore", "--worktree", "--", "."]); }
        Action::TrashUntrackedFiles => return Err("OS Trash handling did not run".to_string()),
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().map_err(|error| format!("Could not run Git: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() { format!("Git action failed: {}", action.label()) } else { detail });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if matches!(action, Action::ViewStash) && stdout.is_empty() {
        Ok("No stashes".to_string())
    } else if stdout.is_empty() {
        Ok(format!("{} complete", action.label()))
    } else {
        Ok(stdout)
    }
}

const PROJECT_PANEL_TRASH_FILE_LIMIT: usize = 1000;
const PROJECT_PANEL_TRASH_STATUS_LIMIT: usize = 10_000;

fn trash_untracked_files(repo_root: &Path) -> Result<String, String> {
    let repo = git2::Repository::discover(repo_root).map_err(|error| format!("Could not find Git repository: {error}"))?;
    let workdir = repo.workdir().ok_or_else(|| "Git repository has no worktree".to_string())?;
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false);
    let statuses = repo.statuses(Some(&mut options)).map_err(|error| format!("Could not list untracked files: {error}"))?;
    if statuses.len() > PROJECT_PANEL_TRASH_STATUS_LIMIT {
        return Err(format!("Too many changed paths to scan safely. Limit: {}", PROJECT_PANEL_TRASH_STATUS_LIMIT));
    }
    let paths: Vec<PathBuf> = statuses.iter()
        .filter(|entry| entry.status().contains(git2::Status::WT_NEW))
        .take(PROJECT_PANEL_TRASH_FILE_LIMIT + 1)
        .filter_map(|entry| entry.path().ok().map(PathBuf::from))
        .collect();
    if paths.len() > PROJECT_PANEL_TRASH_FILE_LIMIT {
        return Err(format!("Too many untracked files to move safely. Limit: {}", PROJECT_PANEL_TRASH_FILE_LIMIT));
    }
    if paths.is_empty() { return Ok("No untracked files".to_string()); }
    let mut moved = 0usize;
    for relative in &paths {
        let full_path = workdir.join(relative);
        if let Err(error) = trash::delete(&full_path) {
            return Err(format!("Moved {moved} files to Trash. Could not move {}: {error}", relative.display()));
        }
        moved += 1;
    }
    Ok(format!("Moved {moved} untracked files to Trash"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub id: String,
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub refs: Vec<String>,
    pub parents: Vec<String>,
    pub parents_truncated: bool,
}

const PROJECT_PANEL_COMMIT_LIMIT: usize = 40;
const PROJECT_PANEL_COMMIT_PARENT_LIMIT: usize = 8;
const PROJECT_PANEL_CHANGE_LIMIT: usize = 100;
const PROJECT_PANEL_REFERENCE_LIMIT: usize = 120;
const PROJECT_PANEL_REFS_PER_COMMIT_LIMIT: usize = 4;
const PROJECT_PANEL_DIFF_FILE_LIMIT: usize = 40;
const PROJECT_PANEL_DIFF_FILE_SIZE_LIMIT: usize = 256 * 1024;
const PROJECT_PANEL_UNTRACKED_FILE_LIMIT: usize = 40;
const PROJECT_PANEL_UNTRACKED_SIZE_LIMIT: u64 = 128 * 1024;
const PROJECT_PANEL_UNTRACKED_WALK_LIMIT: usize = 512;

pub type GitInfo = GitStatus;

impl GitStatus {

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

pub fn list_local_branches(cwd: &Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::discover(cwd) else {
        return Vec::new();
    };
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for item in branches.flatten() {
        if let Ok(Some(name)) = item.0.name() {
            names.push(name.to_string());
        }
    }
    names.sort();
    names
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

    // An initialized repository can have a symbolic HEAD before its first commit.
    let head = repo.head().ok();
    let is_detached = head.as_ref().map(|head| !head.is_branch()).unwrap_or(false);
    let branch = head.as_ref().and_then(|head| head.shorthand().ok()).map(str::to_string)
        .or_else(|| repo.find_reference("HEAD").ok().and_then(|head| {
            head.symbolic_target().ok().flatten().and_then(|target| target.rsplit('/').next().map(str::to_string))
        }))
        .unwrap_or_else(|| "HEAD".to_string());
    let commit = head.and_then(|head| head.peel_to_commit().ok());
    let hash = commit.as_ref().map(|commit| commit.id().to_string()).unwrap_or_default();
    let hash_short = hash.chars().take(7).collect::<String>();
    let summary = commit.as_ref().and_then(|commit| commit.summary().ok().flatten()).unwrap_or("").to_string();
    
    // Ahead/behind vs remote
    let (ahead, behind) = get_ahead_behind(&repo, &branch);
    
    // File status counts
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    let project_panel = fetch_project_panel_snapshot(&repo, &statuses);
    
    let mut staged = 0usize;
    let mut unstaged = 0usize;
    let untracked = project_panel.untracked_file_count;
    
    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW | git2::Status::INDEX_MODIFIED | 
            git2::Status::INDEX_DELETED | git2::Status::INDEX_RENAMED
        ) { staged += 1; }
        if s.intersects(
            git2::Status::WT_MODIFIED | git2::Status::WT_DELETED
        ) { unstaged += 1; }
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
        .and_then(|r| r.url().ok().map(|s| s.to_string()));

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
        last_commit_summary: if hash.is_empty() { String::new() } else { format!("{} {}", &hash[..hash.len().min(7)], summary) },
        remote_url,
        project_panel,
    })
}

fn fetch_project_panel_snapshot(
    repo: &git2::Repository,
    statuses: &git2::Statuses<'_>,
) -> GitPanelSnapshot {
    let mut changes_by_path = HashMap::<String, GitFileChange>::new();
    let mut diff_paths = Vec::new();
    let mut untracked_paths = Vec::new();
    let mut untracked_file_count = 0usize;
    let mut changes_truncated = statuses.iter().nth(PROJECT_PANEL_CHANGE_LIMIT).is_some();
    for entry in statuses.iter().take(PROJECT_PANEL_CHANGE_LIMIT) {
        let status = entry.status();
        let head_delta = entry.head_to_index();
        let worktree_delta = entry.index_to_workdir();
        let path = worktree_delta
            .and_then(|delta| delta.new_file().path().or(delta.old_file().path()))
            .or_else(|| head_delta.and_then(|delta| delta.new_file().path().or(delta.old_file().path())))
            .map(|path| path.to_string_lossy().into_owned());
        let Some(path) = path else { continue };
        let index = map_git_file_side(status, true);
        let worktree = map_git_file_side(status, false);
        if status.contains(git2::Status::WT_NEW) {
            if let Some(workdir) = repo.workdir() {
                let full_path = workdir.join(&path);
                if std::fs::metadata(&full_path).map(|metadata| metadata.is_dir()).unwrap_or(false) {
                    let (files, visited, walk_truncated) = collect_untracked_files(repo, workdir, &path, PROJECT_PANEL_UNTRACKED_WALK_LIMIT - untracked_file_count);
                    untracked_file_count += files.len();
                    changes_truncated |= walk_truncated;
                    for file_path in files {
                        if changes_by_path.len() >= PROJECT_PANEL_CHANGE_LIMIT { changes_truncated = true; break; }
                        untracked_paths.push(file_path.clone());
                        changes_by_path.insert(file_path.clone(), GitFileChange {
                            path: file_path,
                            index: None,
                            worktree: Some(GitFileSide { status: GitFileStatus::Untracked, added: 0, removed: 0 }),
                        });
                    }
                    if visited >= PROJECT_PANEL_UNTRACKED_WALK_LIMIT { break; }
                    continue;
                }
            }
            untracked_file_count = untracked_file_count.saturating_add(1).min(PROJECT_PANEL_UNTRACKED_WALK_LIMIT);
            untracked_paths.push(path.clone());
        }
        if index.is_some() || worktree.is_some() {
            if changes_by_path.len() >= PROJECT_PANEL_CHANGE_LIMIT {
                changes_truncated = true;
                continue;
            }
            changes_by_path.insert(path.clone(), GitFileChange { path: path.clone(), index, worktree });
            diff_paths.push(path.clone());
        }
    }
    let mut index_line_counts = HashMap::<String, (usize, usize)>::new();
    let mut worktree_line_counts = HashMap::<String, (usize, usize)>::new();
    if !diff_paths.is_empty() {
        let mut index_diff_opts = git2::DiffOptions::new();
        index_diff_opts.max_size(PROJECT_PANEL_DIFF_FILE_SIZE_LIMIT as i64).disable_pathspec_match(true);
        for path in diff_paths.iter().take(PROJECT_PANEL_DIFF_FILE_LIMIT) {
            index_diff_opts.pathspec(path);
        }
        if let Some(tree) = repo.head().ok().and_then(|head| head.peel_to_tree().ok()) {
            if let Ok(diff) = repo.diff_tree_to_index(Some(&tree), None, Some(&mut index_diff_opts)) {
                collect_diff_line_counts(&diff, &mut index_line_counts, PROJECT_PANEL_DIFF_FILE_LIMIT);
            }
        }
        let mut worktree_diff_opts = git2::DiffOptions::new();
        worktree_diff_opts.max_size(PROJECT_PANEL_DIFF_FILE_SIZE_LIMIT as i64).disable_pathspec_match(true);
        for path in diff_paths.iter().take(PROJECT_PANEL_DIFF_FILE_LIMIT) {
            worktree_diff_opts.pathspec(path);
        }
        if let Ok(diff) = repo.diff_index_to_workdir(None, Some(&mut worktree_diff_opts)) {
            collect_diff_line_counts(&diff, &mut worktree_line_counts, PROJECT_PANEL_DIFF_FILE_LIMIT);
        }
    }
    for (path, change) in &mut changes_by_path {
        if let Some(side) = change.index.as_mut() {
            if let Some((added, removed)) = index_line_counts.get(path) {
                side.added = *added;
                side.removed = *removed;
            }
        }
        if let Some(side) = change.worktree.as_mut() {
            if let Some((added, removed)) = worktree_line_counts.get(path) {
                side.added = *added;
                side.removed = *removed;
            }
        }
    }
    if let Some(workdir) = repo.workdir() {
        for path in untracked_paths.iter().take(PROJECT_PANEL_UNTRACKED_FILE_LIMIT) {
            let full_path = workdir.join(path);
            let Ok(metadata) = std::fs::metadata(&full_path) else { continue };
            if metadata.len() > PROJECT_PANEL_UNTRACKED_SIZE_LIMIT { continue; }
            let Ok(content) = std::fs::read_to_string(full_path) else { continue };
            if let Some(change) = changes_by_path.get_mut(path) {
                if let Some(side) = change.worktree.as_mut() {
                    side.added = content.lines().count();
                }
            }
        }
    }
    let mut changes: Vec<_> = changes_by_path.into_values().collect();
    changes.sort_by(|left, right| left.path.cmp(&right.path));

    let page = fetch_commit_page(repo, 0);
    GitPanelSnapshot { availability: GitPanelAvailability::Ready, changes, commits: page.commits, history_has_more: page.has_more, untracked_file_count, changes_truncated }
}

fn fetch_commit_page(repo: &git2::Repository, offset: usize) -> GitCommitPage {
    let mut refs_by_commit: HashMap<git2::Oid, Vec<String>> = HashMap::new();
    if let Ok(references) = repo.references() {
        for reference in references.flatten().take(PROJECT_PANEL_REFERENCE_LIMIT) {
            let Ok(name) = reference.shorthand() else { continue };
            let Some(target) = reference.target() else { continue };
            let labels = refs_by_commit.entry(target).or_default();
            if labels.len() < PROJECT_PANEL_REFS_PER_COMMIT_LIMIT { labels.push(name.to_string()); }
        }
    }
    let Ok(mut walk) = repo.revwalk() else { return GitCommitPage { commits: Vec::new(), has_more: false } };
    let mut graph_data_incomplete = walk.push_head().is_err()
        || walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).is_err();
    let mut ids = walk.skip(offset).take(PROJECT_PANEL_COMMIT_LIMIT + 1).collect::<Vec<_>>();
    let has_more = ids.len() > PROJECT_PANEL_COMMIT_LIMIT;
    ids.truncate(PROJECT_PANEL_COMMIT_LIMIT);
    let mut commits = Vec::with_capacity(ids.len());
    for oid in ids {
        let Ok(oid) = oid else { graph_data_incomplete = true; continue };
        let Ok(commit) = repo.find_commit(oid) else { graph_data_incomplete = true; continue };
        let hash = oid.to_string();
        let timestamp = commit.time().seconds();
        let parent_count = commit.parent_count();
        let parents: Vec<_> = (0..parent_count.min(PROJECT_PANEL_COMMIT_PARENT_LIMIT))
            .filter_map(|index| commit.parent_id(index).ok().map(|parent| parent.to_string()))
            .collect();
        commits.push(GitCommit {
            id: hash.clone(),
            hash: hash[..hash.len().min(7)].to_string(),
            subject: commit.summary().ok().flatten().unwrap_or("").to_string(),
            author: commit.author().name().unwrap_or("Unknown").to_string(),
            date: format_commit_date(timestamp),
            refs: refs_by_commit.remove(&oid).unwrap_or_default(),
            parents_truncated: parents.len() != parent_count.min(PROJECT_PANEL_COMMIT_PARENT_LIMIT)
                || parent_count > PROJECT_PANEL_COMMIT_PARENT_LIMIT,
            parents,
        });
    }
    if graph_data_incomplete { for commit in &mut commits { commit.parents_truncated = true; } }
    GitCommitPage { commits, has_more }
}

fn collect_untracked_files(repo: &git2::Repository, workdir: &Path, root: &str, limit: usize) -> (Vec<String>, usize, bool) {
    let mut files = Vec::new();
    let mut pending = VecDeque::from([PathBuf::from(root)]);
    let mut visited = 0usize;
    let mut truncated = false;
    while let Some(relative) = pending.pop_front() {
        if visited >= PROJECT_PANEL_UNTRACKED_WALK_LIMIT || files.len() >= limit { truncated = true; break; }
        visited += 1;
        if repo.status_should_ignore(&relative).unwrap_or(false) { continue; }
        let full = workdir.join(&relative);
        let Ok(entries) = std::fs::read_dir(full) else { continue };
        for entry in entries.flatten() {
            if visited >= PROJECT_PANEL_UNTRACKED_WALK_LIMIT { truncated = true; break; }
            visited += 1;
            let Ok(file_type) = entry.file_type() else { continue };
            let Ok(relative_path) = entry.path().strip_prefix(workdir).map(Path::to_path_buf) else { continue };
            if repo.status_should_ignore(&relative_path).unwrap_or(false) { continue; }
            if file_type.is_dir() {
                pending.push_back(relative_path);
            } else if file_type.is_file() {
                files.push(relative_path.to_string_lossy().into_owned());
                if files.len() >= limit { truncated = true; break; }
            }
        }
    }
    (files, visited, truncated)
}

fn map_git_file_side(status: git2::Status, index: bool) -> Option<GitFileSide> {
    let (conflicted, added, modified, deleted, renamed, typechange) = if index {
        (git2::Status::CONFLICTED, git2::Status::INDEX_NEW, git2::Status::INDEX_MODIFIED,
         git2::Status::INDEX_DELETED, git2::Status::INDEX_RENAMED, git2::Status::INDEX_TYPECHANGE)
    } else {
        (git2::Status::CONFLICTED, git2::Status::WT_NEW, git2::Status::WT_MODIFIED,
         git2::Status::WT_DELETED, git2::Status::WT_RENAMED, git2::Status::WT_TYPECHANGE)
    };
    let state = if status.intersects(conflicted) {
        GitFileStatus::Conflicted
    } else if status.intersects(renamed) {
        GitFileStatus::Renamed
    } else if status.intersects(deleted) {
        GitFileStatus::Deleted
    } else if status.intersects(added) {
        if !index && status.contains(git2::Status::WT_NEW) {
            GitFileStatus::Untracked
        } else {
            GitFileStatus::Added
        }
    } else if status.intersects(typechange) || status.intersects(modified) {
        GitFileStatus::Modified
    } else {
        return None;
    };
    Some(GitFileSide { status: state, added: 0, removed: 0 })
}

fn format_commit_date(timestamp: i64) -> String {
    let age = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp.max(0) as u64))
        .unwrap_or_default();
    let days = age.as_secs() / 86_400;
    match days {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        days if days < 30 => format!("{} days ago", days),
        days if days < 365 => format!("{} months ago", days / 30),
        days => format!("{} years ago", days / 365),
    }
}

fn collect_diff_line_counts(
    diff: &git2::Diff<'_>,
    counts: &mut HashMap<String, (usize, usize)>,
    file_limit: usize,
) {
    let delta_paths: Vec<_> = diff
        .deltas()
        .take(file_limit)
        .filter_map(|delta| delta.new_file().path().or(delta.old_file().path()).map(|path| path.to_string_lossy().into_owned()))
        .collect();
    if delta_paths.is_empty() { return; }
    let _ = diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        let path = delta.new_file().path().or(delta.old_file().path());
        if let Some(path) = path {
            let path = path.to_string_lossy().into_owned();
            if !delta_paths.iter().any(|candidate| candidate == &path) { return true; }
            let entry = counts.entry(path).or_default();
            match line.origin() {
                '+' => entry.0 += 1,
                '-' => entry.1 += 1,
                _ => {}
            }
        }
        true
    });
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

/// TTL for [`fetch_git_info_cached`]. A full status scan (git2 discover +
/// head + recursive untracked walk) is the most expensive recurring read in
/// the app; events like `CwdChanged` + `CommandFinished` often arrive in
/// bursts for the same directory, so a short TTL collapses them into one.
const GIT_INFO_CACHE_TTL: Duration = Duration::from_secs(2);
const GIT_INFO_CACHE_MAX_ENTRIES: usize = 128;

/// Caller path -> (fetch time, status). `None` statuses are cached
/// too so plain directories skip the discover walk-up on every event.
#[derive(Clone)]
struct GitInfoCacheEntry {
    fetched_at: Instant,
    generation: u64,
    info: Option<GitInfo>,
    panel_availability: GitPanelAvailability,
}

type GitInfoCache = Mutex<HashMap<PathBuf, GitInfoCacheEntry>>;
type GitRefreshesInFlight = Mutex<HashMap<PathBuf, u64>>;
type GitInfoGenerations = Mutex<HashMap<PathBuf, u64>>;
type GitGraphPagesInFlight = Mutex<HashSet<PathBuf>>;

#[derive(Debug)]
struct GitCommitPage {
    commits: Vec<GitCommit>,
    has_more: bool,
}

#[derive(Clone, Debug)]
pub enum CachedGitInfo {
    CacheMiss,
    Available { info: GitInfo, stale: bool },
    NoProject,
    NoRepository { stale: bool },
    FetchFailed { stale: bool },
}

fn git_info_cache() -> &'static GitInfoCache {
    static CACHE: OnceLock<GitInfoCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn git_refreshes_in_flight() -> &'static GitRefreshesInFlight {
    static REFRESHES: OnceLock<GitRefreshesInFlight> = OnceLock::new();
    REFRESHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn git_info_generations() -> &'static GitInfoGenerations {
    static GENERATIONS: OnceLock<GitInfoGenerations> = OnceLock::new();
    GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn git_graph_pages_in_flight() -> &'static GitGraphPagesInFlight {
    static PAGES: OnceLock<GitGraphPagesInFlight> = OnceLock::new();
    PAGES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn current_git_info_generation(repo_path: &Path) -> u64 {
    git_info_generations().lock().ok().and_then(|generations| generations.get(repo_path).copied()).unwrap_or(0)
}

pub fn is_git_info_generation_current(repo_path: &Path, generation: u64) -> bool {
    git_info_generations().lock().ok().map(|generations| generations.get(repo_path).copied().unwrap_or(0) == generation).unwrap_or(false)
}

fn store_git_info_if_current(
    repo_path: &Path,
    generation: u64,
    mut info: Option<GitInfo>,
    panel_availability: GitPanelAvailability,
) -> bool {
    let Ok(generations) = git_info_generations().lock() else { return false };
    if generations.get(repo_path).copied().unwrap_or(0) != generation { return false; }
    let Ok(mut cache) = git_info_cache().lock() else { return false };
    if let (Some(previous), Some(refreshed)) = (
        cache.get(repo_path).filter(|entry| entry.generation == generation).and_then(|entry| entry.info.as_ref()),
        info.as_mut(),
    ) {
        preserve_loaded_graph_history(previous, refreshed);
    }
    if cache.len() >= GIT_INFO_CACHE_MAX_ENTRIES { cache.clear(); }
    cache.insert(repo_path.to_path_buf(), GitInfoCacheEntry {
        fetched_at: Instant::now(),
        generation,
        info,
        panel_availability,
    });
    true
}

fn preserve_loaded_graph_history(previous: &GitInfo, refreshed: &mut GitInfo) {
    let previous_graph = &previous.project_panel;
    let refreshed_graph = &mut refreshed.project_panel;
    let Some(previous_head) = previous_graph.commits.first() else { return };
    if !refreshed_graph.commits.iter().any(|commit| commit.id == previous_head.id) { return; }

    let mut known_commits: HashSet<_> = refreshed_graph.commits.iter().map(|commit| commit.id.clone()).collect();
    refreshed_graph.commits.extend(previous_graph.commits.iter()
        .filter(|commit| known_commits.insert(commit.id.clone()))
        .cloned());
    refreshed_graph.history_has_more &= previous_graph.history_has_more;
}

fn availability_for_info(repo_path: &Path, info: Option<&GitInfo>) -> GitPanelAvailability {
    if info.is_some() {
        GitPanelAvailability::Ready
    } else if git2::Repository::discover(repo_path).is_err() {
        GitPanelAvailability::NoRepository
    } else {
        GitPanelAvailability::FetchFailed
    }
}

/// Cached [`fetch_git_info`], keyed by the caller's path so render-side reads need no filesystem lookup.
///
/// `None` (not a repo) is cached too, so plain directories stop paying a
/// `Repository::discover` walk-up on every event. Entries expire after
/// [`GIT_INFO_CACHE_TTL`]; callers that need an immediate refresh after a
/// known mutation (commit, checkout) should call [`fetch_git_info`] directly.
pub fn fetch_git_info_cached(repo_path: &Path) -> Option<GitInfo> {
    let key = repo_path.to_path_buf();
    let generation = current_git_info_generation(&key);
    if let Ok(guard) = git_info_cache().lock() {
        if let Some(entry) = guard.get(&key) {
            if entry.generation == generation && entry.fetched_at.elapsed() < GIT_INFO_CACHE_TTL {
                return entry.info.clone();
            }
        }
    }
    let info = fetch_git_info(repo_path);
    let panel_availability = availability_for_info(repo_path, info.as_ref());
    store_git_info_if_current(&key, generation, info.clone(), panel_availability).then_some(info).flatten()
}

/// Dispatch a stale Git refresh without doing repository work on the caller's thread.
pub fn request_git_info_refresh(repo_path: &Path, completion: impl FnOnce(Option<GitInfo>, u64) + Send + 'static) -> bool {
    let key = repo_path.to_path_buf();
    let generation = current_git_info_generation(&key);
    if git_info_cache().lock().ok().and_then(|cache| cache.get(&key).map(|entry| entry.generation == generation && entry.fetched_at.elapsed() < GIT_INFO_CACHE_TTL)).unwrap_or(false) {
        return false;
    }
    let Ok(mut in_flight) = git_refreshes_in_flight().lock() else { return false };
    if in_flight.get(&key) == Some(&generation) { return false; }
    in_flight.insert(key.clone(), generation);
    drop(in_flight);
    std::thread::spawn(move || {
        let info = fetch_git_info(&key);
        let panel_availability = availability_for_info(&key, info.as_ref());
        let stored = store_git_info_if_current(&key, generation, info.clone(), panel_availability);
        if let Ok(mut in_flight) = git_refreshes_in_flight().lock() {
            if in_flight.get(&key) == Some(&generation) { in_flight.remove(&key); }
        }
        if stored && is_git_info_generation_current(&key, generation) {
            completion(info, generation);
        }
    });
    true
}

/// Load one bounded older-commit page and append it to the cached snapshot.
pub fn request_git_graph_page(
    repo_path: &Path,
    expected_offset: usize,
    completion: impl FnOnce(bool, u64) + Send + 'static,
) -> bool {
    let key = repo_path.to_path_buf();
    let generation = current_git_info_generation(&key);
    let can_load = git_info_cache().lock().ok().and_then(|cache| cache.get(&key).and_then(|entry| {
        (entry.generation == generation).then(|| entry.info.as_ref()).flatten()
            .map(|info| info.project_panel.history_has_more && info.project_panel.commits.len() == expected_offset)
    })).unwrap_or(false);
    if !can_load { return false; }
    let Ok(mut in_flight) = git_graph_pages_in_flight().lock() else { return false };
    if !in_flight.insert(key.clone()) { return false; }
    drop(in_flight);
    std::thread::spawn(move || {
        let page = git2::Repository::discover(&key).ok().map(|repo| fetch_commit_page(&repo, expected_offset));
        let stored = page.and_then(|page| {
            if !is_git_info_generation_current(&key, generation) { return None; }
            let mut cache = git_info_cache().lock().ok()?;
            let entry = cache.get_mut(&key).filter(|entry| entry.generation == generation)?;
            let info = entry.info.as_mut()?;
            if info.project_panel.commits.len() != expected_offset { return None; }
            info.project_panel.commits.extend(page.commits);
            info.project_panel.history_has_more = page.has_more;
            Some(())
        }).is_some();
        if let Ok(mut in_flight) = git_graph_pages_in_flight().lock() { in_flight.remove(&key); }
        if stored && is_git_info_generation_current(&key, generation) { completion(true, generation); }
    });
    true
}

/// Read a cached Git status without refreshing it.
pub fn cached_git_info(repo_path: Option<&Path>) -> Option<GitInfo> {
    let key = repo_path?.to_path_buf();
    git_info_cache().lock().ok()?.get(&key).and_then(|entry| entry.info.clone())
}

pub fn invalidate_git_info_cache(repo_path: &Path) {
    let Ok(mut generations) = git_info_generations().lock() else { return };
    let next_generation = generations.get(repo_path).copied().unwrap_or(0).wrapping_add(1);
    generations.insert(repo_path.to_path_buf(), next_generation);
    if let Ok(mut cache) = git_info_cache().lock() { cache.remove(repo_path); }
}

/// Read cached Git information while preserving the distinction between a miss and a failed fetch.
pub fn cached_git_info_state(repo_path: Option<&Path>) -> CachedGitInfo {
    let Some(path) = repo_path else { return CachedGitInfo::NoProject };
    let generation = current_git_info_generation(path);
    let Ok(cache) = git_info_cache().lock() else { return CachedGitInfo::FetchFailed { stale: false } };
    let Some(entry) = cache.get(path) else { return CachedGitInfo::CacheMiss };
    if entry.generation != generation { return CachedGitInfo::CacheMiss; }
    let stale = entry.fetched_at.elapsed() >= GIT_INFO_CACHE_TTL;
    match &entry.info {
        Some(info) => CachedGitInfo::Available { info: info.clone(), stale },
        None if entry.panel_availability == GitPanelAvailability::NoRepository => CachedGitInfo::NoRepository { stale },
        None => CachedGitInfo::FetchFailed { stale },
    }
}

/// Read the most recent project-panel snapshot without running Git or filesystem queries.
pub fn cached_project_panel_snapshot(repo_path: Option<&Path>) -> GitPanelSnapshot {
    let Some(repo_path) = repo_path else {
        return GitPanelSnapshot { availability: GitPanelAvailability::NoProject, ..GitPanelSnapshot::default() };
    };
    let key = repo_path.to_path_buf();
    let Ok(cache) = git_info_cache().lock() else {
        return GitPanelSnapshot { availability: GitPanelAvailability::FetchFailed, ..GitPanelSnapshot::default() };
    };
    let Some(entry) = cache.get(&key) else {
        return GitPanelSnapshot::default();
    };
    match &entry.info {
        Some(info) => info.project_panel.clone(),
        None => GitPanelSnapshot {
            availability: entry.panel_availability,
            ..GitPanelSnapshot::default()
        },
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_repo(label: &str) -> (PathBuf, git2::Repository) {
        let serial = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastty-{label}-{serial}"));
        std::fs::create_dir_all(&path).unwrap();
        let repo = git2::Repository::init(&path).unwrap();
        (path, repo)
    }

    fn commit_file(repo: &git2::Repository, path: &str, content: &str, message: &str) {
        std::fs::write(repo.workdir().unwrap().join(path), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Fastty Test", "fastty@example.test").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[]).unwrap();
    }

    #[test]
    fn test_cached_matches_direct() {
        // GitStatus has no PartialEq; compare Debug rendering instead.
        let dir = std::env::temp_dir();
        assert_eq!(
            format!("{:?}", fetch_git_info_cached(&dir)),
            format!("{:?}", fetch_git_info(&dir))
        );
        // Second call serves the cached entry.
        assert_eq!(
            format!("{:?}", fetch_git_info_cached(&dir)),
            format!("{:?}", fetch_git_info(&dir))
        );
    }

    #[test]
    fn test_cached_none_for_missing_path() {
        let missing = std::env::temp_dir().join("fastty-definitely-not-a-repo-12345");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(fetch_git_info_cached(&missing).is_none());
    }

    #[test]
    fn project_panel_status_mapping_keeps_index_and_worktree_separate() {
        assert_eq!(map_git_file_side(git2::Status::INDEX_NEW, true).unwrap().status, GitFileStatus::Added);
        assert_eq!(map_git_file_side(git2::Status::WT_MODIFIED, false).unwrap().status, GitFileStatus::Modified);
        assert_eq!(map_git_file_side(git2::Status::WT_NEW, false).unwrap().status, GitFileStatus::Untracked);
        assert_eq!(map_git_file_side(git2::Status::INDEX_DELETED, true).unwrap().status, GitFileStatus::Deleted);
        assert_eq!(map_git_file_side(git2::Status::INDEX_RENAMED, true).unwrap().status, GitFileStatus::Renamed);
        assert!(map_git_file_side(git2::Status::INDEX_NEW, false).is_none());
        let mixed = git2::Status::INDEX_MODIFIED | git2::Status::WT_MODIFIED;
        assert_eq!(map_git_file_side(mixed, true).unwrap().status, GitFileStatus::Modified);
        assert_eq!(map_git_file_side(mixed, false).unwrap().status, GitFileStatus::Modified);
    }

    #[test]
    fn project_panel_separates_staged_and_worktree_line_counts() {
        let (path, repo) = temporary_repo("panel-mixed");
        commit_file(&repo, "sample.txt", "base\n", "base");
        std::fs::write(path.join("sample.txt"), "base\nstaged\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("sample.txt")).unwrap();
        index.write().unwrap();
        std::fs::write(path.join("sample.txt"), "base\nstaged\nworktree\n").unwrap();

        let info = fetch_git_info(&path).unwrap();
        assert_eq!(info.project_panel.availability, GitPanelAvailability::Ready);
        let file = info.project_panel.changes.iter().find(|change| change.path == "sample.txt").unwrap();
        let staged = file.index.as_ref().unwrap();
        let worktree = file.worktree.as_ref().unwrap();
        assert_eq!((staged.added, staged.removed), (1, 0));
        assert_eq!((worktree.added, worktree.removed), (1, 0));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_reports_staged_deletion_and_rename() {
        let (path, repo) = temporary_repo("panel-renames");
        std::fs::write(path.join("removed.txt"), "removed\n").unwrap();
        std::fs::write(path.join("old-name.txt"), "renamed\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("removed.txt")).unwrap();
        index.add_path(Path::new("old-name.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Fastty Test", "fastty@example.test").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "base", &tree, &[]).unwrap();

        std::fs::remove_file(path.join("removed.txt")).unwrap();
        std::fs::rename(path.join("old-name.txt"), path.join("new-name.txt")).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("removed.txt")).unwrap();
        index.remove_path(Path::new("old-name.txt")).unwrap();
        index.add_path(Path::new("new-name.txt")).unwrap();
        index.write().unwrap();

        let info = fetch_git_info(&path).unwrap();
        assert_eq!(info.project_panel.changes.iter().find(|item| item.path == "removed.txt").unwrap().index.as_ref().unwrap().status, GitFileStatus::Deleted);
        assert_eq!(info.project_panel.changes.iter().find(|item| item.path == "new-name.txt").unwrap().index.as_ref().unwrap().status, GitFileStatus::Renamed);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_unstage_keeps_added_file_visible_as_untracked() {
        let (path, repo) = temporary_repo("panel-unstage-visible");
        commit_file(&repo, "base.txt", "base\n", "base");
        std::fs::write(path.join("new.txt"), "new file\n").unwrap();
        let add = std::process::Command::new("git").arg("-C").arg(&path).args(["add", "--", "new.txt"]).status().unwrap();
        assert!(add.success());
        let staged = fetch_git_info(&path).unwrap().project_panel;
        assert!(staged.changes.iter().any(|change| change.path == "new.txt" && change.index.is_some()));

        run_project_git_action(&path, &ProjectGitAction::UnstagePath("new.txt".to_string())).unwrap();
        let unstaged = fetch_git_info(&path).unwrap().project_panel;
        let file = unstaged.changes.iter().find(|change| change.path == "new.txt").expect("unstaged file remains visible");
        assert!(file.index.is_none());
        assert_eq!(file.worktree.as_ref().unwrap().status, GitFileStatus::Untracked);
    }

    #[test]
    fn project_panel_commit_history_uses_bounded_pages() {
        let (path, repo) = temporary_repo("panel-history-pages");
        for index in 0..(PROJECT_PANEL_COMMIT_LIMIT + 3) {
            std::fs::write(path.join("history.txt"), index.to_string()).unwrap();
            let mut index_data = repo.index().unwrap();
            index_data.add_path(Path::new("history.txt")).unwrap();
            index_data.write().unwrap();
            let tree_id = index_data.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent = repo.head().ok().and_then(|head| head.target()).and_then(|oid| repo.find_commit(oid).ok());
            let parents: Vec<_> = parent.iter().collect();
            let signature = git2::Signature::now("Fastty Test", "fastty@example.test").unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, &format!("commit {index}"), &tree, &parents).unwrap();
        }
        let first = fetch_commit_page(&repo, 0);
        assert_eq!(first.commits.len(), PROJECT_PANEL_COMMIT_LIMIT);
        assert!(first.has_more);
        let second = fetch_commit_page(&repo, first.commits.len());
        assert_eq!(second.commits.len(), 3);
        assert!(!second.has_more);
        assert_ne!(first.commits.last().unwrap().id, second.commits.first().unwrap().id);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn git_refresh_preserves_loaded_graph_pages() {
        let (path, repo) = temporary_repo("refresh-preserves-graph-pages");
        for index in 0..(PROJECT_PANEL_COMMIT_LIMIT + 3) {
            std::fs::write(path.join("history.txt"), index.to_string()).unwrap();
            let mut index_data = repo.index().unwrap();
            index_data.add_path(Path::new("history.txt")).unwrap();
            index_data.write().unwrap();
            let tree_id = index_data.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent = repo.head().ok().and_then(|head| head.target()).and_then(|oid| repo.find_commit(oid).ok());
            let parents: Vec<_> = parent.iter().collect();
            let signature = git2::Signature::now("Fastty Test", "fastty@example.test").unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, &format!("commit {index}"), &tree, &parents).unwrap();
        }

        let generation = current_git_info_generation(&path);
        let initial = fetch_git_info(&path).unwrap();
        let first_page_len = initial.project_panel.commits.len();
        assert!(initial.project_panel.history_has_more);
        assert!(store_git_info_if_current(
            &path,
            generation,
            Some(initial),
            GitPanelAvailability::Ready,
        ));

        let older_page = fetch_commit_page(&repo, first_page_len);
        assert_eq!(older_page.commits.len(), 3);
        let older_commit_id = older_page.commits[0].id.clone();
        {
            let mut cache = git_info_cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let info = cache.get_mut(&path).unwrap().info.as_mut().unwrap();
            info.project_panel.commits.extend(older_page.commits);
            info.project_panel.history_has_more = older_page.has_more;
        }

        let refreshed = fetch_git_info(&path).unwrap();
        assert!(store_git_info_if_current(
            &path,
            generation,
            Some(refreshed),
            GitPanelAvailability::Ready,
        ));

        let cached = cached_git_info(Some(&path)).unwrap();
        assert_eq!(cached.project_panel.commits.len(), first_page_len + 3);
        assert!(cached.project_panel.commits.iter().any(|commit| commit.id == older_commit_id));
        assert!(!cached.project_panel.history_has_more);
        drop(repo);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_expands_bounded_untracked_directories_to_files() {
        let (path, repo) = temporary_repo("panel-untracked-dir");
        commit_file(&repo, "tracked.txt", "base\n", "base");
        std::fs::write(path.join(".gitignore"), "*.log\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(".gitignore")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let signature = git2::Signature::now("Fastty Test", "fastty@example.test").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "ignore logs", &tree, &[&parent]).unwrap();
        let nested = path.join("notes/nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("readme.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(nested.join("ignored.log"), "ignore me\n").unwrap();

        let info = fetch_git_info(&path).unwrap();
        assert_eq!(info.untracked, 1);
        assert_eq!(info.project_panel.untracked_file_count, 1);
        assert!(info.project_panel.changes.iter().all(|change| !change.path.ends_with("/")));
        let file = info.project_panel.changes.iter().find(|change| change.path == "notes/nested/readme.txt").unwrap();
        assert_eq!(file.worktree.as_ref().unwrap().added, 3);
        assert!(info.project_panel.changes.iter().all(|change| !change.path.ends_with("ignored.log")));
        drop(tree);
        drop(parent);
        drop(repo);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_supports_unborn_head() {
        let (path, _repo) = temporary_repo("panel-unborn");
        std::fs::write(path.join("first.txt"), "first\n").unwrap();
        let info = fetch_git_info(&path).unwrap();
        assert_eq!(info.project_panel.availability, GitPanelAvailability::Ready);
        assert!(info.project_panel.commits.is_empty());
        assert!(info.project_panel.changes.iter().any(|change| change.path == "first.txt"));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_marks_status_rows_as_truncated() {
        let (path, repo) = temporary_repo("panel-truncated");
        commit_file(&repo, "tracked.txt", "base\n", "base");
        for index in 0..=PROJECT_PANEL_CHANGE_LIMIT {
            std::fs::write(path.join(format!("new-{index}.txt")), "new\n").unwrap();
        }
        let info = fetch_git_info(&path).unwrap();
        assert_eq!(info.project_panel.changes.len(), PROJECT_PANEL_CHANGE_LIMIT);
        assert!(info.project_panel.changes_truncated);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn cached_git_info_state_distinguishes_miss_and_known_non_repo() {
        let path = std::env::temp_dir().join(format!("fastty-cache-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::CacheMiss));
        assert!(fetch_git_info_cached(&path).is_none());
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::NoRepository { stale: false }));
        assert!(matches!(cached_git_info_state(None), CachedGitInfo::NoProject));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn expired_git_cache_refreshes_in_background_and_completes() {
        let (path, repo) = temporary_repo("expired-cache");
        commit_file(&repo, "sample.txt", "base\n", "base");
        assert!(fetch_git_info_cached(&path).is_some());
        if let Ok(mut cache) = git_info_cache().lock() {
            if let Some(entry) = cache.get_mut(&path) {
                entry.fetched_at = Instant::now() - GIT_INFO_CACHE_TTL - Duration::from_millis(1);
            }
        }
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::Available { stale: true, .. }));
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(request_git_info_refresh(&path, move |_, _| { let _ = tx.send(()); }));
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::Available { stale: false, .. }));
        drop(repo);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn invalidation_rejects_old_in_flight_generation() {
        let path = std::env::temp_dir().join(format!("fastty-generation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let old_generation = current_git_info_generation(&path);
        {
            let mut in_flight = git_refreshes_in_flight().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            in_flight.insert(path.clone(), old_generation);
        }
        invalidate_git_info_cache(&path);
        let new_generation = current_git_info_generation(&path);
        assert_ne!(old_generation, new_generation);
        assert!(!is_git_info_generation_current(&path, old_generation));
        assert!(!store_git_info_if_current(&path, old_generation, None, GitPanelAvailability::NoRepository));
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::CacheMiss));
        assert!(store_git_info_if_current(&path, new_generation, None, GitPanelAvailability::NoRepository));
        assert!(matches!(cached_git_info_state(Some(&path)), CachedGitInfo::NoRepository { stale: false }));
        if let Ok(mut in_flight) = git_refreshes_in_flight().lock() { in_flight.remove(&path); }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn project_panel_distinguishes_clean_unborn_and_no_repo() {
        let (repo_path, _repo) = temporary_repo("panel-empty-repo");
        assert!(fetch_git_info_cached(&repo_path).is_some());
        assert_eq!(cached_project_panel_snapshot(Some(&repo_path)).availability, GitPanelAvailability::Ready);
        assert!(cached_project_panel_snapshot(Some(&repo_path)).commits.is_empty());
        std::fs::remove_dir_all(repo_path).unwrap();

        let (clean_path, repo) = temporary_repo("panel-clean");
        commit_file(&repo, "sample.txt", "clean\n", "clean");
        assert!(fetch_git_info_cached(&clean_path).is_some());
        assert_eq!(cached_project_panel_snapshot(Some(&clean_path)).availability, GitPanelAvailability::Ready);
        assert!(cached_project_panel_snapshot(Some(&clean_path)).changes.is_empty());
        std::fs::remove_dir_all(clean_path).unwrap();

        let no_repo = std::env::temp_dir().join("fastty-panel-no-repo-known");
        let _ = std::fs::remove_dir_all(&no_repo);
        std::fs::create_dir_all(&no_repo).unwrap();
        assert!(fetch_git_info_cached(&no_repo).is_none());
        assert_eq!(cached_project_panel_snapshot(Some(&no_repo)).availability, GitPanelAvailability::NoRepository);
        assert_eq!(cached_project_panel_snapshot(None).availability, GitPanelAvailability::NoProject);
        std::fs::remove_dir_all(no_repo).unwrap();
    }

    #[test]
    fn project_panel_bulk_actions_are_all_labeled_and_confirmation_is_explicit() {
        use ProjectGitAction as Action;
        let actions = [
            Action::UnstageAll,
            Action::RestoreAllChanges,
            Action::StashAll,
            Action::StashTracked,
            Action::StashPop,
            Action::ViewStash,
            Action::DiscardTrackedChanges,
            Action::TrashUntrackedFiles,
        ];
        let labels: Vec<_> = actions.iter().map(ProjectGitAction::label).collect();
        assert_eq!(labels, [
            "Unstage All", "Restore All Changes", "Stash All", "Stash Tracked",
            "Stash Pop", "View Stash", "Discard Tracked Changes", "Trash Untracked Files",
        ]);
        assert_eq!(actions.iter().filter(|action| action.requires_confirmation()).count(), 4);
        assert!(!Action::StageAll.requires_confirmation());
        assert!(!Action::StagePath("file.txt".to_string()).requires_confirmation());
        let trash = Action::TrashUntrackedFiles;
        assert_eq!(trash.label(), "Trash Untracked Files");
        assert!(trash.requires_confirmation());
        assert!(trash.moves_to_os_trash());
        assert_eq!(trash.confirmation_message(), Some("Move untracked files to your system Trash."));
        assert!(!Action::DiscardTrackedChanges.moves_to_os_trash());
    }

    #[test]
    fn project_panel_actions_resolve_subdirectory_to_repository_root() {
        let (repo_path, _repo) = temporary_repo("panel-action-root");
        let nested = repo_path.join("src").join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(project_git_repository_root(&nested).unwrap(), repo_path.canonicalize().unwrap());
        std::fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn project_panel_confirmation_requires_same_tab_and_cwd() {
        let cwd = std::env::temp_dir().join("fastty-project-confirmation-cwd");
        let other_cwd = cwd.join("other");
        let confirmation = ProjectGitConfirmation {
            action: ProjectGitAction::TrashUntrackedFiles,
            tab_id: 7,
            cwd: cwd.clone(),
        };

        assert!(confirmation.matches(7, Some(&cwd)));
        assert!(!confirmation.matches(8, Some(&cwd)));
        assert!(!confirmation.matches(7, Some(&other_cwd)));
        assert!(!confirmation.matches(7, None));
    }
}
