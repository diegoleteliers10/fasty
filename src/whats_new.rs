//! Post-update "What's new" dialog.
//!
//! At startup the running version is compared against the last version seen
//! on this machine (a plain-text file in the state dir). When the running
//! version is newer, the changelog section for it, compiled into the binary
//! from `CHANGELOG.md`, is shown once. An install with no recorded version
//! but existing app state counts as an upgrade too, so machines coming from
//! releases older than the tracker still see the notes. This works for every
//! install channel (self-update, Homebrew, .deb, MSI) because it compares the
//! running binary against local state, not the network.

/// Changelog compiled into the binary: always available, also offline.
const CHANGELOG: &str = include_str!("../CHANGELOG.md");

fn last_seen_version_path() -> std::path::PathBuf {
    crate::paths::get().state_dir.join("last_seen_version")
}

/// Whether the What's new dialog should open on this launch.
///
/// Two cases fire it:
/// 1. The running version is newer than the last version seen on this
///    machine (the normal self-update / package-manager upgrade path).
/// 2. No version was ever recorded AND the machine already has Fastty state.
///    The tracker file only exists since 0.13.0, so an install that jumps
///    from an older release straight past 0.13.0 has no baseline and would
///    otherwise stay silent forever (seen on macOS: 0.12.0 -> 0.13.1 showed
///    nothing while Windows, which had run 0.13.0 first, showed the dialog).
///    Existing state (config file or persisted session) marks those machines
///    as upgraders; a genuinely fresh install has neither and stays silent.
pub fn should_show() -> bool {
    let seen = std::fs::read_to_string(last_seen_version_path())
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|seen| !seen.is_empty());
    match seen {
        Some(seen) => is_upgrade(env!("CARGO_PKG_VERSION"), &seen),
        None => has_existing_install(),
    }
}

fn is_upgrade(current: &str, seen: &str) -> bool {
    match (
        crate::updater::parse_version(current),
        crate::updater::parse_version(seen),
    ) {
        (Some(c), Some(s)) => c > s,
        _ => current != seen,
    }
}

/// `paths::init` creates the Fastty directories on every launch, so their
/// existence says nothing. These files only exist once the app has actually
/// been used on this machine.
fn has_existing_install() -> bool {
    crate::config::Config::get_active_config_path().exists()
        || crate::session::session_path().exists()
}

/// Records the running version as seen. Call once per launch, right after
/// `should_show`, so the dialog fires exactly once per version.
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

    #[test]
    fn upgrade_detection() {
        assert!(is_upgrade("0.13.1", "0.13.0"));
        assert!(is_upgrade("0.14.0", "v0.13.1"));
        assert!(!is_upgrade("0.13.0", "0.13.0"));
        assert!(!is_upgrade("0.12.9", "0.13.0"));
    }
}
