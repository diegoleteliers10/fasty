//! Git status types shared between main and renderer.

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
