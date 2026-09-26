//! Bounded file index and fuzzy search for the file path picker.
//!
//! `build_index` walks a directory tree with hard caps on entries, depth,
//! and time, so opening the picker never stalls the UI thread on huge
//! checkouts. `search` scores entries with a subsequence matcher that
//! favors segment starts, filename hits, and short paths.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Stop indexing at this many entries. Keeps memory and UI work bounded.
pub const MAX_ENTRIES: usize = 20_000;
/// Do not descend deeper than this many levels below the root.
pub const MAX_DEPTH: usize = 12;
/// Give up on the walk after this long and serve what we have.
pub const TIME_BUDGET: Duration = Duration::from_millis(150);
/// Cap on rows returned by [`search`].
pub const MAX_RESULTS: usize = 50;

/// Directory names the index never offers or descends into.
const SKIPPED_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "node_modules", "target", "dist", "build", "out",
    "vendor", ".venv", "venv", "__pycache__", ".cache", "coverage", ".next",
    ".turbo", ".parcel-cache", "DerivedData", "Pods", ".build", ".idea",
    ".gradle", ".dart_tool", "site-packages", ".terraform",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the search root, `/`-separated.
    pub rel_path: String,
    pub is_dir: bool,
}

impl FileEntry {
    /// Final path segment.
    pub fn file_name(&self) -> &str {
        self.rel_path.rsplit('/').next().unwrap_or(&self.rel_path)
    }
}

/// Walks `root` and returns a bounded, deterministic file index.
///
/// Dot directories are skipped (dot files stay, they are common paste
/// targets). Symlinks are never followed, which also prevents cycles.
pub fn build_index(root: &Path) -> Vec<FileEntry> {
    let mut index = Vec::new();
    let deadline = Instant::now() + TIME_BUDGET;
    // (directory, relative prefix below root, depth below root)
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        if index.len() >= MAX_ENTRIES || Instant::now() > deadline || depth > MAX_DEPTH {
            continue;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = read.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            if index.len() >= MAX_ENTRIES {
                break;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = file_type.is_dir();
            if is_dir && (name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_str())) {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if is_dir {
                stack.push((entry.path(), rel.clone(), depth + 1));
            }
            index.push(FileEntry { rel_path: rel, is_dir });
        }
    }
    index
}

/// Ranks `index` against `query` and returns at most `limit` entries.
///
/// An empty query returns the head of the index. Ordering is stable:
/// score desc, then shorter path, then lexicographic.
pub fn search<'a>(index: &'a [FileEntry], query: &str, limit: usize) -> Vec<&'a FileEntry> {
    let query = query.trim();
    if query.is_empty() {
        return index.iter().take(limit).collect();
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(i64, &'a FileEntry)> = index
        .iter()
        .filter_map(|e| score_match(&e.rel_path.to_lowercase(), &q).map(|s| (s, e)))
        .collect();
    scored.sort_by(|a, b| {
        b.0
            .cmp(&a.0)
            .then(a.1.rel_path.len().cmp(&b.1.rel_path.len()))
            .then(a.1.rel_path.cmp(&b.1.rel_path))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, e)| e).collect()
}

/// Subsequence score for a lowercased path against lowercased query chars.
/// Returns None when the query is not a subsequence of the path.
fn score_match(lower_path: &str, query: &[char]) -> Option<i64> {
    let mut query_chars = query.iter().copied();
    let mut want = query_chars.next();
    let mut score = 0i64;
    let mut prev_matched: Option<usize> = None;
    let mut prev_char: Option<char> = None;
    let filename_start = lower_path.rfind('/').map(|i| i + 1).unwrap_or(0);
    for (i, ch) in lower_path.chars().enumerate() {
        let Some(qc) = want else { break };
        if ch != qc {
            prev_char = Some(ch);
            continue;
        }
        score += match prev_matched {
            Some(prev) if prev + 1 == i => 8,
            Some(_) => 2,
            None => 3,
        };
        let at_segment_start = i == 0 || matches!(prev_char, Some('/') | Some('_') | Some('-') | Some('.'));
        if at_segment_start {
            score += 6;
        }
        if i >= filename_start {
            score += 3;
        }
        prev_matched = Some(i);
        prev_char = Some(ch);
        want = query_chars.next();
    }
    if want.is_none() {
        Some(score - lower_path.chars().count() as i64 / 6)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry { rel_path: path.to_string(), is_dir }
    }

    #[test]
    fn build_index_skips_dot_and_junk_dirs_but_keeps_dotfiles() {
        let root = std::env::temp_dir().join(format!("fastty_fs_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/left-pad")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::write(root.join(".gitignore"), "").unwrap();

        let index = build_index(&root);

        let paths: Vec<&str> = index.iter().map(|e| e.rel_path.as_str()).collect();
        assert!(paths.contains(&"src"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&".gitignore"));
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
        assert!(!paths.iter().any(|p| p.contains(".git/")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_empty_query_returns_index_head() {
        let index = vec![entry("a.txt", false), entry("b.txt", false)];
        let got = search(&index, "  ", 10);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn search_ranks_filename_segment_hits_first() {
        let index = vec![
            entry("docs/license-notes.md", false),
            entry("apps/license-lookup-app/src/types/License.ts", false),
            entry("license", true),
        ];
        let got = search(&index, "licens", 10);
        assert!(!got.is_empty());
        // Shortest, highest-scoring entry wins.
        assert_eq!(got[0].rel_path, "license");
        // Subsequence across separators still matches.
        let got2 = search(&index, "lts", 10);
        assert!(got2.iter().any(|e| e.rel_path.ends_with("License.ts")));
    }

    #[test]
    fn search_returns_nothing_when_not_a_subsequence() {
        let index = vec![entry("src/main.rs", false)];
        assert!(search(&index, "zzz", 10).is_empty());
    }
}
