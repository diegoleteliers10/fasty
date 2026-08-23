// @env agnostic

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Decodes a percent-encoded string (for example "%20" to " ").
pub fn percent_decode_str(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = [c1, c2];
                if let Ok(hex_val) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(hex_val, 16) {
                        bytes.push(byte);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(c1);
                bytes.push(c2);
            } else {
                bytes.push(b'%');
                if let Some(c1) = h1 {
                    bytes.push(c1);
                }
            }
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// Normalizes a single `file://` URI into a local `PathBuf`.
pub fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if !trimmed.starts_with("file://") {
        return None;
    }

    let without_scheme = &trimmed["file://".len()..];
    let path_str = if without_scheme.starts_with("localhost/") {
        &without_scheme["localhost".len()..]
    } else {
        without_scheme
    };

    let decoded = percent_decode_str(path_str);

    #[cfg(target_os = "windows")]
    let clean_path = {
        let trimmed_leading = decoded.trim_start_matches('/');
        PathBuf::from(trimmed_leading)
    };

    #[cfg(not(windows))]
    let clean_path = PathBuf::from(&decoded);

    Some(clean_path)
}

/// Formats a filesystem path into a shell-safe representation.
pub fn format_path_for_shell(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains(|c: char| c.is_whitespace() || "\\'\"$`!*?()[]{}|;&<>~".contains(c)) {
        format!("'{}'", s.replace('\'', r"'\''"))
    } else {
        s.into_owned()
    }
}

/// Formats multiple paths for shell usage separated by space.
pub fn format_paths_for_shell(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format_path_for_shell(p))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses clipboard text that may contain file URIs or plain text.
pub fn parse_file_uris_or_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return text.to_string();
    }

    let all_file_uris = lines.iter().all(|l| l.starts_with("file://"));
    if all_file_uris {
        let paths: Vec<PathBuf> = lines.iter().filter_map(|l| decode_file_uri(l)).collect();
        if !paths.is_empty() {
            return format_paths_for_shell(&paths);
        }
    }

    text.to_string()
}

/// Cleans up old temporary clipboard images older than 24 hours.
fn prune_old_temp_images(temp_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(temp_dir) {
        let now = SystemTime::now();
        let max_age = Duration::from_secs(24 * 3600);
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > max_age {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
}

/// Saves image data from clipboard to a temporary PNG file.
pub fn save_clipboard_image(image_data: arboard::ImageData) -> Result<PathBuf, String> {
    let base_temp = std::env::temp_dir().join("fastty_clipboard");
    if !base_temp.exists() {
        let _ = std::fs::create_dir_all(&base_temp);
    }

    prune_old_temp_images(&base_temp);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = format!("fastty_clip_{}_{}.png", timestamp, count);
    let file_path = base_temp.join(file_name);

    let width = image_data.width as u32;
    let height = image_data.height as u32;
    let raw_bytes = image_data.bytes.into_owned();

    let img_buffer = image::RgbaImage::from_raw(width, height, raw_bytes)
        .ok_or_else(|| "Failed to construct RGBA image buffer from clipboard".to_string())?;

    img_buffer
        .save_with_format(&file_path, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to save clipboard image to disk: {}", e))?;

    Ok(file_path)
}

/// Reads paste content from system clipboard (text or image).
pub fn get_clipboard_paste_content(clip: &mut arboard::Clipboard) -> Option<String> {
    // 1. Try reading text first
    if let Ok(text) = clip.get_text() {
        if !text.is_empty() {
            return Some(parse_file_uris_or_text(&text));
        }
    }

    // 2. If no text or text is empty, check for image data
    if let Ok(image_data) = clip.get_image() {
        if let Ok(saved_path) = save_clipboard_image(image_data) {
            return Some(format_path_for_shell(&saved_path));
        }
    }

    None
}

/// Writes paste text into the terminal PTY with bracketed paste if enabled.
pub fn paste_text_to_terminal(terminal: &crate::terminal_state::TerminalState, text: &str) {
    if terminal.is_bracketed_paste_enabled() {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend_from_slice(text.as_bytes());
        buf.extend_from_slice(b"\x1b[201~");
        terminal.write_to_pty(&buf);
    } else {
        terminal.write_to_pty(text.as_bytes());
    }
}

/// Handles external paths dropped into the terminal.
pub fn handle_dropped_paths(
    terminal: &crate::terminal_state::TerminalState,
    paths: &[PathBuf],
) {
    let formatted = format_paths_for_shell(paths);
    if !formatted.is_empty() {
        paste_text_to_terminal(terminal, &formatted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode_str() {
        assert_eq!(percent_decode_str("hello%20world"), "hello world");
        assert_eq!(percent_decode_str("path%2Fwith%2Fslashes"), "path/with/slashes");
        assert_eq!(percent_decode_str("no_percent"), "no_percent");
    }

    #[test]
    fn test_decode_file_uri() {
        let uri = "file:///Users/user/Desktop/test%20image.png";
        let path = decode_file_uri(uri).unwrap();
        #[cfg(not(windows))]
        assert_eq!(path.to_string_lossy(), "/Users/user/Desktop/test image.png");

        let uri_localhost = "file://localhost/tmp/test.png";
        let path_localhost = decode_file_uri(uri_localhost).unwrap();
        #[cfg(not(windows))]
        assert_eq!(path_localhost.to_string_lossy(), "/tmp/test.png");
    }

    #[test]
    fn test_format_path_for_shell() {
        assert_eq!(format_path_for_shell(Path::new("/tmp/test.png")), "/tmp/test.png");
        assert_eq!(
            format_path_for_shell(Path::new("/tmp/test space.png")),
            "'/tmp/test space.png'"
        );
        assert_eq!(
            format_path_for_shell(Path::new("/tmp/it's.png")),
            r"'/tmp/it'\''s.png'"
        );
    }

    #[test]
    fn test_format_paths_for_shell() {
        let paths = vec![
            PathBuf::from("/tmp/a.png"),
            PathBuf::from("/tmp/b c.png"),
        ];
        assert_eq!(
            format_paths_for_shell(&paths),
            "/tmp/a.png '/tmp/b c.png'"
        );
    }

    #[test]
    fn test_parse_file_uris_or_text() {
        let uris = "file:///tmp/img1.png\nfile:///tmp/img2.png";
        let result = parse_file_uris_or_text(uris);
        assert_eq!(result, "/tmp/img1.png /tmp/img2.png");

        let normal_text = "echo 'hello'";
        let result_normal = parse_file_uris_or_text(normal_text);
        assert_eq!(result_normal, "echo 'hello'");
    }

    #[test]
    fn test_save_clipboard_image() {
        let width = 2;
        let height = 2;
        let bytes = vec![
            255, 0, 0, 255,
            0, 255, 0, 255,
            0, 0, 255, 255,
            255, 255, 0, 255,
        ];
        let image_data = arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(bytes),
        };
        let saved_path = save_clipboard_image(image_data).expect("Must save image");
        assert!(saved_path.exists());
        let _ = std::fs::remove_file(saved_path);
    }
}
