//! Post-update "What's new" dialog.
//!
//! At startup the running version is compared against the last version seen
//! on this machine (a plain-text file in the state dir). When the running
//! version is newer, the changelog section for it, compiled into the binary
//! from `CHANGELOG.md`, is shown once. This works for every install channel
//! (self-update, Homebrew, .deb, MSI) because it compares the running binary
//! against local state, not the network.

/// Changelog compiled into the binary: always available, also offline.
const CHANGELOG: &str = include_str!("../CHANGELOG.md");

fn last_seen_version_path() -> std::path::PathBuf {
    crate::paths::get().state_dir.join("last_seen_version")
}

/// `Some(previous)` when the running version is newer than the last version
/// seen on this machine. `None` on first launch and on downgrades.
pub fn pending_upgrade() -> Option<String> {
    let seen = std::fs::read_to_string(last_seen_version_path()).ok()?;
    let seen = seen.trim();
    if seen.is_empty() {
        return None;
    }
    let current = env!("CARGO_PKG_VERSION");
    let is_upgrade = match (
        crate::updater::parse_version(current),
        crate::updater::parse_version(seen),
    ) {
        (Some(c), Some(s)) => c > s,
        _ => current != seen,
    };
    is_upgrade.then(|| seen.to_string())
}

/// Records the running version as seen. Call once per launch, right after
/// `pending_upgrade`, so the dialog fires exactly once per version.
pub fn mark_version_seen() {
    let path = last_seen_version_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, env!("CARGO_PKG_VERSION")).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The `CHANGELOG.md` section for `version` (heading `## <version> ...`),
/// without the heading itself. `None` when the section does not exist.
pub fn notes_for(version: &str) -> Option<String> {
    section_for(CHANGELOG, version)
}

fn section_for(changelog: &str, version: &str) -> Option<String> {
    let wanted = version.trim().trim_start_matches('v');
    let mut in_section = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if in_section {
                break;
            }
            in_section = heading.split_whitespace().next() == Some(wanted);
        } else if in_section {
            lines.push(line);
        }
    }
    let notes = lines.join("\n").trim().to_string();
    (!notes.is_empty()).then_some(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_section_and_stops_at_next_heading() {
        let md = "# Changelog\n\n## 1.2.0 - 2026-01-01\n\n- A\n- B\n\n## 1.1.0 - 2025-12-01\n\n- C\n";
        assert_eq!(section_for(md, "1.2.0").as_deref(), Some("- A\n- B"));
        assert_eq!(section_for(md, "v1.1.0").as_deref(), Some("- C"));
    }

    #[test]
    fn missing_or_empty_section_is_none() {
        let md = "## 1.2.0 - 2026-01-01\n\n- A\n\n## 1.0.0\n\n\n";
        assert_eq!(section_for(md, "9.9.9"), None);
        assert_eq!(section_for(md, "1.0.0"), None);
    }
}
