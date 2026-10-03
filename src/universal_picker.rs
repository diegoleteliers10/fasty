//! Multi-source fuzzy insert picker: "insert a path to anything, anywhere".
//!
//! Extends the bounded workspace file index with SSH hosts, local git
//! branches, snippets, and running docker containers, so one anchored picker
//! can drop any of them straight into the focused terminal's prompt. The
//! picker complements the shell instead of hijacking it: picks only insert
//! text (paths quoted, commands pre-filled with a trailing space), never
//! execute. Inspired by Superlogical's universal path insertion, see
//! docs/plans/ideas-superlogical.md item 12.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Cap on rows returned by [`search`].
pub const MAX_RESULTS: usize = 50;

/// Rows of each non-file source shown in the empty-query head.
const EMPTY_QUERY_KIND_QUOTA: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Dir,
    SshHost,
    GitBranch,
    Snippet,
    DockerContainer,
}

impl ItemKind {
    /// Tiny lowercase tag shown on picker rows of this kind.
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::File => "file",
            ItemKind::Dir => "dir",
            ItemKind::SshHost => "ssh",
            ItemKind::GitBranch => "branch",
            ItemKind::Snippet => "snippet",
            ItemKind::DockerContainer => "docker",
        }
    }

    /// Small bonus so hits from the (small) non-file sources surface
    /// alongside the (much larger) file index when scores are close.
    fn base_score(self) -> i64 {
        match self {
            ItemKind::File | ItemKind::Dir => 0,
            ItemKind::SshHost | ItemKind::GitBranch | ItemKind::DockerContainer => 2,
            ItemKind::Snippet => 1,
        }
    }

    /// Deterministic ordering for score ties.
    fn rank(self) -> u8 {
        match self {
            ItemKind::File => 0,
            ItemKind::Dir => 1,
            ItemKind::SshHost => 2,
            ItemKind::GitBranch => 3,
            ItemKind::Snippet => 4,
            ItemKind::DockerContainer => 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UniversalItem {
    pub kind: ItemKind,
    /// Primary label: file name, ssh alias, branch name, snippet trigger.
    pub title: String,
    /// Secondary muted label: directory prefix, user@host, image name.
    pub detail: String,
    /// Text inserted at the terminal prompt when picked.
    pub insert: String,
}

/// Builds every picker source for the focused workspace.
///
/// Files come first so [`search`]'s empty-query head can cap them in a
/// single pass and still pick up the other kinds behind them.
pub fn build_sources(root: Option<&Path>, current_branch: Option<&str>) -> Vec<UniversalItem> {
    let mut items: Vec<UniversalItem> = Vec::new();

    if let Some(root) = root {
        for entry in crate::file_search::build_index(root) {
            let file_name = entry.file_name().to_string();
            let dir_prefix = entry.rel_path[..entry.rel_path.len() - file_name.len()].to_string();
            items.push(UniversalItem {
                kind: if entry.is_dir { ItemKind::Dir } else { ItemKind::File },
                title: file_name,
                detail: dir_prefix,
                insert: crate::paste::format_path_for_shell(Path::new(&entry.rel_path)),
            });
        }
        for branch in crate::git::list_local_branches(root) {
            if current_branch == Some(branch.as_str()) {
                continue;
            }
            items.push(UniversalItem {
                kind: ItemKind::GitBranch,
                detail: "git checkout".to_string(),
                insert: format!("git checkout {branch} "),
                title: branch,
            });
        }
    }

    for host in crate::ssh::parse_ssh_config() {
        items.push(UniversalItem {
            kind: ItemKind::SshHost,
            title: host.name.clone(),
            detail: host.display(),
            insert: format!("ssh {} ", host.name),
        });
    }

    for (trigger, body) in crate::snippets::all() {
        let (expanded, _) = crate::snippets::expand(&body);
        let detail: String = expanded
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(48)
            .collect();
        items.push(UniversalItem {
            kind: ItemKind::Snippet,
            title: trigger,
            detail,
            insert: expanded,
        });
    }

    for container in docker_containers() {
        items.push(UniversalItem {
            kind: ItemKind::DockerContainer,
            title: container.name.clone(),
            detail: container.image,
            insert: format!("docker exec -it {} ", container.name),
        });
    }

    items
}

/// Ranks `items` against `query` and returns at most `limit` items.
///
/// An empty query serves the head of the file index plus a small quota of
/// every other source, so the picker opens with a useful mix.
pub fn search<'a>(items: &'a [UniversalItem], query: &str, limit: usize) -> Vec<&'a UniversalItem> {
    let query = query.trim();
    if query.is_empty() {
        return empty_query_head(items, limit);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(i64, &UniversalItem)> = items
        .iter()
        .filter_map(|it| {
            let haystack = format!("{} {}", it.title, it.detail).to_lowercase();
            score_text(&haystack, &q).map(|s| (s + it.kind.base_score(), it))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.title.len().cmp(&b.1.title.len()))
            .then((a.1.kind.rank(), &a.1.title).cmp(&(b.1.kind.rank(), &b.1.title)))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, it)| it).collect()
}

fn empty_query_head<'a>(items: &'a [UniversalItem], limit: usize) -> Vec<&'a UniversalItem> {
    let mut out: Vec<&'a UniversalItem> = Vec::new();
    // Reserve room for a small quota of every non-file source at the end.
    let file_cap = limit.saturating_sub(EMPTY_QUERY_KIND_QUOTA * 4);
    let mut taken: [usize; 6] = [0; 6];
    for item in items {
        if out.len() >= limit {
            break;
        }
        match item.kind {
            ItemKind::File | ItemKind::Dir => {
                if out.len() < file_cap {
                    out.push(item);
                }
            }
            other => {
                let slot = other.rank() as usize;
                if taken[slot] < EMPTY_QUERY_KIND_QUOTA {
                    taken[slot] += 1;
                    out.push(item);
                }
            }
        }
    }
    out
}

/// Subsequence score of lowercased `text` against lowercased query chars,
/// favoring consecutive runs and word starts. Returns None when the query is
/// not a subsequence of the text. Same shape as `file_search::score_match`,
/// without the filename-region bias.
fn score_text(text: &str, query: &[char]) -> Option<i64> {
    let mut query_chars = query.iter().copied();
    let mut want = query_chars.next();
    let mut score = 0i64;
    let mut prev_matched: Option<usize> = None;
    let mut prev_char: Option<char> = None;
    for (i, ch) in text.chars().enumerate() {
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
        let at_word_start = i == 0
            || matches!(
                prev_char,
                Some('/') | Some('_') | Some('-') | Some('.') | Some(' ') | Some('@') | Some(':')
            );
        if at_word_start {
            score += 6;
        }
        prev_matched = Some(i);
        prev_char = Some(ch);
        want = query_chars.next();
    }
    if want.is_none() {
        Some(score - text.chars().count() as i64 / 6)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct DockerContainer {
    pub name: String,
    pub image: String,
}

struct DockerSnapshot {
    fetched_at: Instant,
    containers: Vec<DockerContainer>,
}

const DOCKER_TTL: Duration = Duration::from_secs(30);
static DOCKER_CACHE: OnceLock<Mutex<Option<DockerSnapshot>>> = OnceLock::new();
static DOCKER_REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Cached running containers. Refreshes out of band so a slow or missing
/// docker daemon never blocks the picker from opening; the first open after
/// launch simply has no docker rows until the background fetch lands.
pub fn docker_containers() -> Vec<DockerContainer> {
    let cache = DOCKER_CACHE.get_or_init(|| Mutex::new(None));
    let fresh = {
        let guard = cache.lock().unwrap();
        match guard.as_ref() {
            Some(snap) if snap.fetched_at.elapsed() < DOCKER_TTL => {
                Some(snap.containers.clone())
            }
            _ => None,
        }
    };
    if let Some(containers) = fresh {
        return containers;
    }
    spawn_docker_refresh();
    // Serve stale rows while the refresh runs; empty until the first fetch.
    cache
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.containers.clone())
        .unwrap_or_default()
}

fn spawn_docker_refresh() {
    if DOCKER_REFRESH_IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let containers = list_docker_containers_blocking();
        let cache = DOCKER_CACHE.get_or_init(|| Mutex::new(None));
        *cache.lock().unwrap() = Some(DockerSnapshot {
            fetched_at: Instant::now(),
            containers,
        });
        DOCKER_REFRESH_IN_FLIGHT.store(false, Ordering::Relaxed);
    });
}

fn list_docker_containers_blocking() -> Vec<DockerContainer> {
    let mut cmd = std::process::Command::new("docker");
    cmd.args(["ps", "--format", "{{.Names}}\t{{.Image}}"]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_docker_ps(&String::from_utf8_lossy(&out.stdout))
}

/// Parses `docker ps --format '{{.Names}}\t{{.Image}}'` output.
fn parse_docker_ps(output: &str) -> Vec<DockerContainer> {
    output
        .lines()
        .filter_map(|line| {
            let (name, image) = line.split_once('\t')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(DockerContainer {
                name: name.to_string(),
                image: image.trim().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: ItemKind, title: &str, detail: &str) -> UniversalItem {
        UniversalItem {
            kind,
            title: title.to_string(),
            detail: detail.to_string(),
            insert: title.to_string(),
        }
    }

    fn sample() -> Vec<UniversalItem> {
        vec![
            item(ItemKind::File, "main.rs", "src/"),
            item(ItemKind::File, "config.rs", "src/"),
            item(ItemKind::File, "notes.md", "docs/"),
            item(ItemKind::GitBranch, "feature-picker", "git checkout"),
            item(ItemKind::SshHost, "prod-api", "deploy@10.0.0.1"),
            item(ItemKind::Snippet, "todo", "// TODO: fix"),
            item(ItemKind::DockerContainer, "pg-dev", "postgres:16"),
        ]
    }

    #[test]
    fn empty_query_serves_files_then_quota_of_each_kind() {
        let items = sample();
        let got = search(&items, "", 50);
        // All three files fit under the file cap...
        assert!(got.iter().any(|i| i.title == "main.rs"));
        // ...and every non-file source shows up too.
        assert!(got.iter().any(|i| i.kind == ItemKind::GitBranch));
        assert!(got.iter().any(|i| i.kind == ItemKind::SshHost));
        assert!(got.iter().any(|i| i.kind == ItemKind::Snippet));
        assert!(got.iter().any(|i| i.kind == ItemKind::DockerContainer));
        // Files stay first in the mix.
        assert_eq!(got[0].kind, ItemKind::File);
    }

    #[test]
    fn empty_query_caps_non_file_quota() {
        let items: Vec<UniversalItem> = (0..10)
            .map(|i| item(ItemKind::SshHost, &format!("host{i}"), ""))
            .collect();
        let got = search(&items, "", 50);
        assert_eq!(got.len(), EMPTY_QUERY_KIND_QUOTA);
    }

    #[test]
    fn query_matches_title_and_detail_across_kinds() {
        let items = sample();
        // "prod" hits the ssh alias and its user@host detail.
        let got = search(&items, "prod", 50);
        assert!(got.iter().any(|i| i.kind == ItemKind::SshHost && i.title == "prod-api"));
        // "checkout" only lives in the branch detail.
        let got = search(&items, "checkout", 50);
        assert!(got.iter().all(|i| i.kind == ItemKind::GitBranch));
        // Subsequence across the word still matches.
        let got = search(&items, "fpick", 50);
        assert!(got.iter().any(|i| i.title == "feature-picker"));
    }

    #[test]
    fn query_returns_nothing_when_not_a_subsequence() {
        let items = sample();
        assert!(search(&items, "zzz", 50).is_empty());
    }

    #[test]
    fn parse_docker_ps_handles_tabs_and_junk() {
        let out = "pg-dev\tpostgres:16\n  web \t nginx:alpine \n\nbroken\n";
        let got = parse_docker_ps(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "pg-dev");
        assert_eq!(got[0].image, "postgres:16");
        assert_eq!(got[1].name, "web");
        assert_eq!(got[1].image, "nginx:alpine");
    }
}
