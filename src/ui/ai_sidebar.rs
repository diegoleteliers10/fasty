use gpui::*;
use gpui::prelude::*;
use std::f32::consts::PI;
use std::time::Duration;
use icons::common::IconType;
use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub struct AiUiToolCall {
    pub id: String,
    pub name: String,
    pub args: String,
    pub output: Option<String>,
    pub is_error: bool,
    pub is_running: bool,
    /// Applied diff for edit tools, rendered as a review card under the row.
    pub diff: Option<crate::ai::diff::FileDiff>,
}

#[derive(Debug, Clone)]
pub struct AiUiMessage {
    pub is_user: bool,
    pub text: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<AiUiToolCall>,
    pub timestamp: Option<String>,
    pub images: Vec<std::path::PathBuf>,
    pub documents: Vec<std::path::PathBuf>,
}

impl AiUiMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            is_user: true,
            text: text.into(),
            thinking: None,
            tool_calls: Vec::new(),
            timestamp: None,
            images: Vec::new(),
            documents: Vec::new(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            is_user: false,
            text: text.into(),
            thinking: None,
            tool_calls: Vec::new(),
            timestamp: None,
            images: Vec::new(),
            documents: Vec::new(),
        }
    }

    pub fn with_images(mut self, images: Vec<std::path::PathBuf>) -> Self {
        self.images = images;
        self
    }

    pub fn with_documents(mut self, documents: Vec<std::path::PathBuf>) -> Self {
        self.documents = documents;
        self
    }

    pub fn with_thinking(mut self, thinking: impl Into<String>) -> Self {
        self.thinking = Some(thinking.into());
        self
    }

    pub fn with_tool_calls(mut self, calls: Vec<AiUiToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    pub fn with_timestamp(mut self, ts: impl Into<String>) -> Self {
        self.timestamp = Some(ts.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct AiUiPendingConfirmation {
    pub tool_id: String,
    pub tool_name: String,
    pub input_summary: String,
}

pub type MouseDownCallback = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
pub type ConfirmCallback = Box<dyn Fn(&(bool, bool), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct AiSidebar {
    pub theme: Theme,
    pub width: f32,
    pub provider_name: String,
    pub model_name: String,
    pub cwd: String,
    pub git_branch: Option<String>,
    pub context_pct: f32,
    pub agent_mode: String,
    pub messages: Vec<AiUiMessage>,
    pub streaming_text: String,
    pub streaming_thinking: String,
    pub is_streaming: bool,
    pub is_focused: bool,
    pub pending_confirmation: Option<AiUiPendingConfirmation>,
    pub input_text: String,
    pub cursor_pos: usize,
    pub selection: Option<(usize, usize)>,
    pub on_close: Option<MouseDownCallback>,
    pub on_clear: Option<MouseDownCallback>,
    pub on_cancel: Option<MouseDownCallback>,
    pub on_submit: Option<MouseDownCallback>,
    pub on_confirm: Option<ConfirmCallback>,
    pub on_focus: Option<MouseDownCallback>,
    pub on_click_char: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    pub on_new_chat: Option<MouseDownCallback>,
    pub on_model_click: Option<MouseDownCallback>,
    pub on_toggle_mode: Option<Box<dyn Fn(&String, &mut Window, &mut App) + 'static>>,
    pub expanded_thinkings: std::collections::HashSet<usize>,
    pub on_toggle_thinking: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    pub attached_files: Vec<std::path::PathBuf>,
    pub on_remove_attachment: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    pub on_at_click: Option<MouseDownCallback>,
    pub on_attach_click: Option<MouseDownCallback>,
    pub at_menu_open: bool,
    pub at_matches: Vec<String>,
    pub on_select_at_match: Option<Box<dyn Fn(&String, &mut Window, &mut App) + 'static>>,
    pub composer_bounds: Option<std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>>,
    pub message_selection: Option<(usize, usize, usize)>,
    pub on_select_message_char: Option<std::sync::Arc<dyn Fn(&(usize, usize), &mut Window, &mut App) + 'static>>,
    pub on_drag_message_char: Option<std::sync::Arc<dyn Fn(&(usize, usize), &mut Window, &mut App) + 'static>>,
    pub scroll_handle: Option<gpui::ScrollHandle>,
    /// True while the message "Copy" button shows the "Copied" confirmation.
    pub copy_feedback: bool,
    pub on_copied: Option<MouseDownCallback>,
}

impl AiSidebar {
    pub fn new(
        theme: Theme,
        width: f32,
        provider_name: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            theme,
            width,
            provider_name: provider_name.into(),
            model_name: model_name.into(),
            cwd: "~".to_string(),
            git_branch: None,
            context_pct: 0.15,
            agent_mode: "Agent".to_string(),
            messages: Vec::new(),
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            is_streaming: false,
            is_focused: true,
            pending_confirmation: None,
            input_text: String::new(),
            cursor_pos: 0,
            selection: None,
            on_close: None,
            on_clear: None,
            on_cancel: None,
            on_submit: None,
            on_confirm: None,
            on_focus: None,
            on_click_char: None,
            on_new_chat: None,
            on_model_click: None,
            on_toggle_mode: None,
            expanded_thinkings: std::collections::HashSet::new(),
            on_toggle_thinking: None,
            attached_files: Vec::new(),
            on_remove_attachment: None,
            on_at_click: None,
            on_attach_click: None,
            at_menu_open: false,
            at_matches: Vec::new(),
            on_select_at_match: None,
            composer_bounds: None,
            message_selection: None,
            on_select_message_char: None,
            on_drag_message_char: None,
            scroll_handle: None,
            copy_feedback: false,
            on_copied: None,
        }
    }

    pub fn scroll_handle(mut self, handle: gpui::ScrollHandle) -> Self {
        self.scroll_handle = Some(handle);
        self
    }

    pub fn copy_feedback(mut self, active: bool) -> Self {
        self.copy_feedback = active;
        self
    }

    pub fn on_copied(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_copied = Some(Box::new(handler));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn git_branch(mut self, branch: Option<String>) -> Self {
        self.git_branch = branch;
        self
    }

    pub fn context_pct(mut self, pct: f32) -> Self {
        self.context_pct = pct.clamp(0.0, 1.0);
        self
    }

    pub fn agent_mode(mut self, mode: impl Into<String>) -> Self {
        self.agent_mode = mode.into();
        self
    }

    pub fn cursor_pos(mut self, pos: usize) -> Self {
        self.cursor_pos = pos;
        self
    }

    pub fn selection(mut self, sel: Option<(usize, usize)>) -> Self {
        self.selection = sel;
        self
    }

    pub fn on_click_char(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_click_char = Some(Box::new(handler));
        self
    }

    pub fn messages(mut self, messages: Vec<AiUiMessage>) -> Self {
        self.messages = messages;
        self
    }

    pub fn streaming(
        mut self,
        is_streaming: bool,
        text: impl Into<String>,
        thinking: impl Into<String>,
    ) -> Self {
        self.is_streaming = is_streaming;
        self.streaming_text = text.into();
        self.streaming_thinking = thinking.into();
        self
    }

    pub fn pending_confirmation(mut self, conf: Option<AiUiPendingConfirmation>) -> Self {
        self.pending_confirmation = conf;
        self
    }

    pub fn input_text(mut self, text: impl Into<String>) -> Self {
        self.input_text = text.into();
        self
    }

    pub fn composer_bounds(mut self, bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>) -> Self {
        self.composer_bounds = Some(bounds);
        self
    }

    pub fn message_selection(mut self, sel: Option<(usize, usize, usize)>) -> Self {
        self.message_selection = sel;
        self
    }

    pub fn on_select_message_char(
        mut self,
        cb: impl Fn(&(usize, usize), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select_message_char = Some(std::sync::Arc::new(cb));
        self
    }

    pub fn on_drag_message_char(
        mut self,
        cb: impl Fn(&(usize, usize), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drag_message_char = Some(std::sync::Arc::new(cb));
        self
    }

    pub fn on_close(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Box::new(handler));
        self
    }

    pub fn on_clear(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_clear = Some(Box::new(handler));
        self
    }

    pub fn on_cancel(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Box::new(handler));
        self
    }

    pub fn on_submit(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Box::new(handler));
        self
    }

    pub fn on_confirm(
        mut self,
        handler: impl Fn(&(bool, bool), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_confirm = Some(Box::new(handler));
        self
    }

    pub fn is_focused(mut self, focused: bool) -> Self {
        self.is_focused = focused;
        self
    }

    pub fn on_focus(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_focus = Some(Box::new(handler));
        self
    }

    pub fn on_new_chat(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_new_chat = Some(Box::new(handler));
        self
    }

    pub fn on_model_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_model_click = Some(Box::new(handler));
        self
    }

    pub fn on_toggle_mode(
        mut self,
        handler: impl Fn(&String, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_mode = Some(Box::new(handler));
        self
    }

    pub fn expanded_thinkings(mut self, set: std::collections::HashSet<usize>) -> Self {
        self.expanded_thinkings = set;
        self
    }

    pub fn on_toggle_thinking(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_thinking = Some(Box::new(handler));
        self
    }

    pub fn attached_files(mut self, files: Vec<std::path::PathBuf>) -> Self {
        self.attached_files = files;
        self
    }

    pub fn on_remove_attachment(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_remove_attachment = Some(Box::new(handler));
        self
    }

    pub fn on_at_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_at_click = Some(Box::new(handler));
        self
    }

    pub fn on_attach_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_attach_click = Some(Box::new(handler));
        self
    }

    pub fn at_menu_open(mut self, open: bool) -> Self {
        self.at_menu_open = open;
        self
    }

    pub fn at_matches(mut self, matches: Vec<String>) -> Self {
        self.at_matches = matches;
        self
    }

    pub fn on_select_at_match(
        mut self,
        handler: impl Fn(&String, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select_at_match = Some(Box::new(handler));
        self
    }
}

pub fn is_image_path(path: &std::path::Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "ico"
        )
    } else {
        false
    }
}

pub fn pick_file_dialog() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"try
set f to choose file with prompt "Select Image or File"
return POSIX path of f
on error
return ""
end try"#;
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("zenity")
            .args(["--file-selection", "--title=Select Image or File"])
            .output()
        {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(std::path::PathBuf::from(path_str));
                }
            }
        }
        if let Ok(output) = std::process::Command::new("kdialog")
            .args(["--getopenfilename", "."])
            .output()
        {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(std::path::PathBuf::from(path_str));
                }
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        let ps_cmd = "[System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms') | Out-Null; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Title = 'Select Image or File'; if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.FileName }";
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", ps_cmd])
            .output()
            .ok()?;
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn format_cwd_display(cwd: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = std::path::Path::new(cwd).strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    cwd.to_string()
}

pub fn current_time_str() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (secs / 3600 % 24) as u32;
    let mins = (secs / 60 % 60) as u32;
    let (h12, ampm) = if hours == 0 {
        (12, "AM")
    } else if hours < 12 {
        (hours, "AM")
    } else if hours == 12 {
        (12, "PM")
    } else {
        (hours - 12, "PM")
    };
    format!("{}:{:02} {}", h12, mins, ampm)
}

struct ParsedToolRow {
    icon_type: IconType,
    action: &'static str,
    target: String,
    status_text: String,
    is_running: bool,
    is_error: bool,
}

fn parse_tool_row(tc: &AiUiToolCall) -> ParsedToolRow {
    let (icon_type, action) = match tc.name.as_str() {
        "run_command" => (IconType::Terminal, "Run"),
        "search" | "list_dir" => (IconType::Search, "Search"),
        "read_file" => (IconType::FileCode, "Read"),
        "edit_file" => (IconType::Pencil, "Edit"),
        _ => (IconType::Sparkles, "Tool"),
    };

    let raw_target = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&tc.args) {
        if let Some(cmd) = val.get("command").and_then(|v| v.as_str()) {
            cmd.to_string()
        } else if let Some(path) = val.get("path").and_then(|v| v.as_str()) {
            path.to_string()
        } else if let Some(pattern) = val.get("pattern").and_then(|v| v.as_str()) {
            pattern.to_string()
        } else if let Some(query) = val.get("query").and_then(|v| v.as_str()) {
            query.to_string()
        } else {
            tc.args.clone()
        }
    } else {
        tc.args.clone()
    };

    let cleaned_target = raw_target.replace('\n', " ");
    let max_len = 28;
    let target = if cleaned_target.chars().count() > max_len {
        let truncated: String = cleaned_target.chars().take(max_len).collect();
        format!("{}...", truncated.trim_end())
    } else {
        cleaned_target
    };

    let status_text = if tc.is_running {
        "running".to_string()
    } else if tc.is_error {
        // run_command errors carry "Command exited with code N:\n..." prefix
        if tc.name == "run_command" {
            if let Some(ref out) = tc.output {
                let first = out.lines().next().unwrap_or("");
                if let Some(rest) = first.strip_prefix("Command exited with code ") {
                    let code = rest.trim_end_matches(':').trim();
                    format!("exit {}", code)
                } else {
                    "failed".to_string()
                }
            } else {
                "failed".to_string()
            }
        } else {
            "failed".to_string()
        }
    } else if let Some(ref out) = tc.output {
        if tc.name == "run_command" {
            if out.contains("test result: ok") {
                "ok".to_string()
            } else {
                "done".to_string()
            }
        } else if tc.name == "search" || tc.name == "list_dir" {
            // First line is "Found N matches:" or "Found N matching files ..."
            let first = out.lines().next().unwrap_or("");
            if let Some(rest) = first.strip_prefix("Found ") {
                let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if n.is_empty() { "done".to_string() } else { format!("{} found", n) }
            } else if first.starts_with("No matches") || first.starts_with("Directory") {
                let lines = out.lines().count().saturating_sub(1);
                if lines > 0 { format!("{} entries", lines) } else { "0 found".to_string() }
            } else {
                let count = out.lines().count();
                format!("{} results", count)
            }
        } else if tc.name == "read_file" {
            // Header: "--- path (lines S-E of T) ---"
            let first = out.lines().next().unwrap_or("");
            if let Some(inner) = first.rfind("(lines ").and_then(|i| {
                let s = &first[i + 7..];
                s.find(" of ").map(|e| s[..e].to_string())
            }) {
                format!("L{}", inner)
            } else {
                let lines = out.lines().count();
                format!("L1-{}", lines)
            }
        } else if tc.name == "edit_file" {
            // "Successfully replaced 1 occurrence in '...' (+5 -3)"
            // "Successfully wrote N bytes to '...'"
            // Use rfind("(+") to specifically target stats paren, not parens in file paths.
            if let Some(paren_start) = out.rfind("(+").or_else(|| out.rfind("(-")) {
                let inner = &out[paren_start + 1..];
                if let Some(paren_end) = inner.find(')') {
                    inner[..paren_end].trim().to_string()
                } else {
                    "done".to_string()
                }
            } else if out.contains("wrote") {
                let words: Vec<&str> = out.split_whitespace().collect();
                if let Some(pos) = words.iter().position(|w| *w == "wrote") {
                    words.get(pos + 1).map(|n| format!("{} B", n)).unwrap_or_else(|| "done".to_string())
                } else {
                    "done".to_string()
                }
            } else {
                "done".to_string()
            }
        } else {
            "done".to_string()
        }
    } else {
        "done".to_string()
    };

    ParsedToolRow {
        icon_type,
        action,
        target,
        status_text,
        is_running: tc.is_running,
        is_error: tc.is_error,
    }
}

#[derive(Debug, Clone)]
enum DiffRow {
    Context(String),
    Del(String),
    Add(String),
}

struct ParsedDiffCard {
    file_path: String,
    add_count: usize,
    del_count: usize,
    rows: Vec<DiffRow>,
}

const DIFF_CONTEXT_LINES: usize = 3;
const DIFF_MAX_ROWS: usize = 200;

/// Diff card rendered under an edit_file tool row. While the tool runs it
/// previews the change from the raw args; once applied it shows the real
/// diff. When this exact tool call is awaiting permission, Accept / Reject
/// buttons render on the card so the user can decide right there.
fn render_tool_diff_card(
    theme: &Theme,
    diff: &crate::ai::diff::FileDiff,
    pending: bool,
    running: bool,
    on_confirm: Option<std::rc::Rc<ConfirmCallback>>,
    card_idx: usize,
) -> Div {
    let file_name = diff
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&diff.path)
        .to_string();

    let mut card = div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(6.))
        .border_1()
        .border_color(if pending { theme.accent } else { theme.border })
        .bg(theme.main_bg)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(8.))
                .py(px(4.))
                .bg(theme.surface)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(if pending { theme.accent } else { theme.muted })
                                .flex_shrink_0()
                                .child(if running {
                                    "Editing"
                                } else if pending {
                                    "Edit — review"
                                } else {
                                    "Edited"
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .child(SharedString::from(file_name)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::BOLD)
                        .flex_shrink_0()
                        .child(
                            div()
                                .text_color(theme.bright_green)
                                .child(SharedString::from(format!("+{}", diff.added))),
                        )
                        .child(
                            div()
                                .text_color(theme.bright_red)
                                .child(SharedString::from(format!("-{}", diff.removed))),
                        ),
                ),
        );

    // Diff body as a SINGLE text element inside the scroll box: per-row divs
    // collapsed on top of each other inside the scrollable flex container,
    // so the whole diff is painted as one StyledText with per-line
    // highlights instead. Short diffs size naturally; long ones get a
    // fixed-height inner scroll box.
    //
    // Each line shows a muted gutter with the real file line number (not the
    // "Linea N" text, and baked-in read_file "NNN |" prefixes are hidden from
    // display), then the +/-/space marker and the content.
    let mut body_text = String::new();
    let mut body_highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    for line in &diff.lines {
        let num = line.old_line.or(line.new_line);
        let gutter = match num {
            Some(n) => format!("{:>4} ", n),
            None => "     ".to_string(),
        };
        let (prefix, color, tint) = match line.kind {
            crate::ai::diff::DiffLineKind::Context => (" ", theme.muted_strong, None),
            crate::ai::diff::DiffLineKind::Del => {
                ("-", theme.bright_red, Some(gpui::Hsla::from(gpui::rgba(0xf04e4e1a))))
            }
            crate::ai::diff::DiffLineKind::Add => (
                "+",
                theme.bright_green,
                Some(gpui::Hsla::from(gpui::rgba(0x8ee0441a))),
            ),
        };
        let gutter_start = body_text.len();
        body_text.push_str(&gutter);
        let content_start = body_text.len();
        body_text.push_str(prefix);
        body_text.push_str(crate::ai::diff::strip_baked_prefix(&line.text));
        let content_end = body_text.len();
        body_text.push('\n');
        body_highlights.push((
            gutter_start..content_start,
            HighlightStyle {
                color: Some(theme.muted),
                ..Default::default()
            },
        ));
        let mut style = HighlightStyle {
            color: Some(color),
            ..Default::default()
        };
        if let Some(tint) = tint {
            style.background_color = Some(tint);
        }
        body_highlights.push((content_start..content_end, style));
    }
    if body_text.ends_with('\n') {
        body_text.pop();
    }

    let tall = diff.lines.len() > 12;
    let mut body = div()
        .id(ElementId::named_usize("ai-tool-diff-body", card_idx))
        .w_full()
        .px(px(4.))
        .py(px(4.))
        .text_size(px(11.))
        .whitespace_nowrap()
        .overflow_hidden()
        // Occlude the panel scroll behind this body: wheel gestures over the
        // diff scroll only the diff, never the messages area (GPUI applies
        // wheel deltas to every scrollable under the cursor otherwise).
        .occlude();
    if tall {
        body = body.h(px(240.)).overflow_y_scroll();
    }
    if diff.lines.is_empty() {
        body = body.child(
            div()
                .px(px(3.))
                .py(px(2.))
                .text_size(px(11.))
                .text_color(theme.muted)
                .child(if running { "Preparing diff…" } else { "No changes" }),
        );
    } else {
        body = body.child(StyledText::new(body_text).with_highlights(body_highlights));
    }
    card = card.child(body);

    // Inline Accept / Allow Always / Reject while awaiting permission for
    // this exact call. Allow Always registers the file path in the session
    // allowlist so later edits to the same file apply without asking.
    if pending {
        if let Some(on_confirm) = on_confirm {
            let on_confirm_allow = on_confirm.clone();
            let on_confirm_always = on_confirm.clone();
            let on_confirm_decline = on_confirm;
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(8.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(render_confirm_button(
                                "Accept", theme, true, false, (false, true), &on_confirm_allow,
                            ))
                            .child(render_confirm_button(
                                "Allow Always",
                                theme,
                                false,
                                false,
                                (true, true),
                                &on_confirm_always,
                            ))
                            .child(render_confirm_button(
                                "Reject", theme, false, true, (false, false), &on_confirm_decline,
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child("⌘↵ Accept"),
                    ),
            );
        }
    }

    card
}

fn render_confirm_button(
    label: &'static str,
    theme: &Theme,
    primary: bool,
    danger: bool,
    decision: (bool, bool),
    on_confirm: &std::rc::Rc<ConfirmCallback>,
) -> Div {
    let on_confirm = on_confirm.clone();
    div()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .text_size(px(11.5))
        .cursor(CursorStyle::PointingHand)
        .map(|this| {
            if primary {
                this.bg(theme.accent)
                    .text_color(gpui::rgb(0x151515))
                    .font_weight(FontWeight::SEMIBOLD)
            } else {
                this.bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .text_color(if danger { theme.bright_red } else { theme.foreground })
                    .hover(move |s| s.bg(theme.hover))
            }
        })
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            on_confirm(&decision, window, cx);
        })
}

fn render_confirmation_section(
    theme: &Theme,
    conf: &AiUiPendingConfirmation,
    on_confirm: Option<std::rc::Rc<ConfirmCallback>>,
) -> AnyElement {
    let Some(on_confirm) = on_confirm else {
        return div().into_any_element();
    };

    // edit_file confirmations render inline on the tool row's diff card
    // (Accept / Reject buttons next to the diff), never as a separate card.
    if conf.tool_name == "edit_file" {
        return div().into_any_element();
    }

    if let Some(diff) = parse_diff_confirmation(&conf.tool_name, &conf.input_summary) {
        let on_confirm_allow = on_confirm.clone();
        let on_confirm_decline = on_confirm.clone();
        return div()
            .flex()
            .flex_col()
            .w_full()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_raised)
            .overflow_hidden()
            // File header: path + change stats
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .min_w(px(0.))
                            .child(crate::ui::icons::render_icon(
                                IconType::FileCode,
                                theme.muted,
                                13.0,
                            ))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .overflow_hidden()
                                    .child(SharedString::from(diff.file_path)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .flex_shrink_0()
                            .child(
                                div()
                                    .text_color(theme.bright_green)
                                    .child(SharedString::from(format!("+{}", diff.add_count))),
                            )
                            .child(
                                div()
                                    .text_color(theme.bright_red)
                                    .child(SharedString::from(format!("-{}", diff.del_count))),
                            ),
                    ),
            )
            // Diff body (scrollable so large edits stay reviewable)
            .child(
                div()
                    .id("ai-confirm-diff-body")
                    .flex()
                    .flex_col()
                    .p(px(10.))
                    .bg(theme.main_bg)
                    .gap(px(2.))
                    .max_h(px(300.))
                    .overflow_y_scroll()
                    .children(diff.rows.into_iter().map(|row| {
                        let (prefix, text, color, tint) = match row {
                            DiffRow::Context(line) => ("  ", line, theme.muted_strong, None),
                            DiffRow::Del(line) => ("- ", line, theme.bright_red, Some(gpui::rgba(0xf04e4e1a))),
                            DiffRow::Add(line) => ("+ ", line, theme.bright_green, Some(gpui::rgba(0x8ee0441a))),
                        };
                        div()
                            .w_full()
                            .px(px(4.))
                            .py(px(1.5))
                            .rounded(px(2.))
                            .text_size(px(11.5))
                            .text_color(color)
                            .overflow_hidden()
                            .when_some(tint, |d, tint| d.bg(tint))
                            .child(SharedString::from(format!("{}{}", prefix, text)))
                    })),
            )
            // Actions: edits are accept/reject — no "Allow Always", every edit
            // keeps showing its diff.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .p(px(10.))
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(render_confirm_button("Accept", theme, true, false, (false, true), &on_confirm_allow))
                            .child(render_confirm_button("Reject", theme, false, true, (false, false), &on_confirm_decline)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child("⌘↵ Accept"),
                    ),
            )
            .into_any_element();
    }

    let on_confirm_allow = on_confirm.clone();
    let on_confirm_always = on_confirm.clone();
    let on_confirm_decline = on_confirm;
    div()
        .flex()
        .flex_col()
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.accent)
        .bg(theme.surface_raised)
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .child(format!("Allow '{}'?", conf.tool_name)),
        )
        .child(
            div()
                .p(px(6.))
                .rounded(px(4.))
                .bg(theme.main_bg)
                .text_size(px(11.5))
                .text_color(theme.foreground)
                .overflow_hidden()
                .child(SharedString::from(conf.input_summary.clone())),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .pt(px(2.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(render_confirm_button("Allow", theme, true, false, (false, true), &on_confirm_allow))
                        .child(render_confirm_button("Allow Always", theme, false, false, (true, true), &on_confirm_always))
                        .child(render_confirm_button("Decline", theme, false, true, (false, false), &on_confirm_decline)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.muted)
                        .child("⌘↵ Allow"),
                ),
        )
        .into_any_element()
}

fn parse_diff_confirmation(tool_name: &str, summary: &str) -> Option<ParsedDiffCard> {
    if tool_name != "edit_file" {
        return None;
    }
    let val: serde_json::Value = serde_json::from_str(summary).ok()?;
    let path = val.get("path")?.as_str()?.to_string();
    let old_s = val.get("old_string").and_then(|v| v.as_str());
    let new_s = val.get("new_string").and_then(|v| v.as_str());
    let content = val.get("content").and_then(|v| v.as_str());

    let mut rows: Vec<DiffRow> = Vec::new();
    let (add_count, del_count) = if let (Some(old_str), Some(new_str)) = (old_s, new_s) {
        let old_lines: Vec<&str> = old_str.lines().collect();
        let new_lines: Vec<&str> = new_str.lines().collect();

        let prefix = old_lines
            .iter()
            .zip(new_lines.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // Compare from the ends so an insertion in the middle keeps the
        // shared tail aligned.
        let suffix = old_lines
            .iter()
            .rev()
            .zip(new_lines.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(old_lines.len() - prefix)
            .min(new_lines.len() - prefix);
        let old_mid = &old_lines[prefix..old_lines.len() - suffix];
        let new_mid = &new_lines[prefix..new_lines.len() - suffix];

        let ctx_start = prefix.saturating_sub(DIFF_CONTEXT_LINES);
        for line in &old_lines[ctx_start..prefix] {
            rows.push(DiffRow::Context((*line).to_string()));
        }
        for line in old_mid {
            rows.push(DiffRow::Del((*line).to_string()));
        }
        for line in new_mid {
            rows.push(DiffRow::Add((*line).to_string()));
        }
        let ctx_end = (new_lines.len() - suffix + DIFF_CONTEXT_LINES).min(new_lines.len());
        for line in &new_lines[new_lines.len() - suffix..ctx_end] {
            rows.push(DiffRow::Context((*line).to_string()));
        }

        (new_mid.len(), old_mid.len())
    } else if let Some(cnt) = content {
        for line in cnt.lines() {
            rows.push(DiffRow::Add(line.to_string()));
        }
        (cnt.lines().count(), 0)
    } else {
        return None;
    };

    if rows.len() > DIFF_MAX_ROWS {
        rows.truncate(DIFF_MAX_ROWS);
    }

    Some(ParsedDiffCard {
        file_path: path,
        add_count,
        del_count,
        rows,
    })
}

const THINKING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_context_ring(pct: f32, theme: &Theme) -> impl IntoElement {
    let progress = pct.clamp(0.0, 1.0);
    let size = px(12.0);
    let stroke_width = px(2.0);
    let radius = (size / 2.0) - stroke_width;
    let bg_color = theme.border;
    let progress_color = if progress > 0.8 { theme.bright_red } else { theme.accent };

    canvas(
        |_, _, _| {},
        move |bounds, _, window, _cx| {
            let center_x = bounds.origin.x + bounds.size.width / 2.0;
            let center_y = bounds.origin.y + bounds.size.height / 2.0;

            let mut bg_builder = PathBuilder::stroke(stroke_width);
            bg_builder.move_to(point(center_x + radius, center_y));
            bg_builder.arc_to(point(radius, radius), px(0.), false, true, point(center_x - radius, center_y));
            bg_builder.arc_to(point(radius, radius), px(0.), false, true, point(center_x + radius, center_y));
            bg_builder.close();
            if let Ok(path) = bg_builder.build() {
                window.paint_path(path, bg_color);
            }

            if progress > 0.0 {
                let mut progress_builder = PathBuilder::stroke(stroke_width);
                if progress >= 0.999 {
                    progress_builder.move_to(point(center_x + radius, center_y));
                    progress_builder.arc_to(point(radius, radius), px(0.), false, true, point(center_x - radius, center_y));
                    progress_builder.arc_to(point(radius, radius), px(0.), false, true, point(center_x + radius, center_y));
                    progress_builder.close();
                } else {
                    progress_builder.move_to(point(center_x, center_y - radius));
                    let angle = -PI / 2.0 + (progress * 2.0 * PI);
                    progress_builder.arc_to(
                        point(radius, radius),
                        px(0.),
                        progress > 0.5,
                        true,
                        point(center_x + radius * angle.cos(), center_y + radius * angle.sin()),
                    );
                }
                if let Ok(path) = progress_builder.build() {
                    window.paint_path(path, progress_color);
                }
            }
        },
    )
    .size(size)
}

fn render_thinking_indicator(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme.accent)
                .with_animation(
                    "ai-thinking-dots",
                    Animation::new(Duration::from_millis(1000))
                        .repeat()
                        .with_max_fps(12.),
                    |el, delta| {
                        let frame =
                            (delta * THINKING_FRAMES.len() as f32) as usize % THINKING_FRAMES.len();
                        el.child(THINKING_FRAMES[frame])
                    },
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(theme.muted)
                .child("Thinking")
                .with_animation(
                    "ai-thinking-pulse",
                    Animation::new(Duration::from_millis(1600))
                        .repeat()
                        .with_easing(pulsating_between(0.45, 1.0))
                        .with_max_fps(30.),
                    |el, delta| el.opacity(delta),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_markdown_prose(
    text: &str,
    theme: &Theme,
    is_streaming: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    window: Option<&Window>,
    registry: Option<(&crate::ui::markdown::RowRegistry, usize)>,
) -> Div {
    crate::ui::markdown::render_markdown_selectable(
        text,
        theme,
        is_streaming,
        selection,
        on_select_char,
        on_drag_char,
        text_left,
        window,
        registry,
    )
}

impl RenderOnce for AiSidebar {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let win_ref = &*window;
        let theme = self.theme;
        let sidebar_bg = Hsla { a: theme.opacity, ..theme.sidebar_bg };
        let surface_raised = theme.surface_raised;

        let is_focused = self.is_focused;
        let on_close = self.on_close.map(std::rc::Rc::new);
        let on_clear = self.on_clear.map(std::rc::Rc::new);
        let on_cancel = self.on_cancel.map(std::rc::Rc::new);
        let on_submit = self.on_submit.map(std::rc::Rc::new);
        let on_confirm = self.on_confirm.map(std::rc::Rc::new);
        let pending_confirmation = self.pending_confirmation.clone();
        let on_copied = self.on_copied.map(std::rc::Rc::new);
        let copy_feedback = self.copy_feedback;
        // Selectable text rows are registered fresh on every render pass.
        let row_registry: crate::ui::markdown::RowRegistry =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let on_focus = self.on_focus.map(std::rc::Rc::new);
        let on_click_char = self.on_click_char.map(std::rc::Rc::new);
        let on_new_chat = self.on_new_chat.map(std::rc::Rc::new);
        let _on_model_click = self.on_model_click.map(std::rc::Rc::new);
        let on_toggle_mode = self.on_toggle_mode.map(std::rc::Rc::new);
        let on_toggle_thinking = self.on_toggle_thinking.map(std::rc::Rc::new);
        let on_remove_attachment = self.on_remove_attachment.map(std::rc::Rc::new);
        let on_at_click = self.on_at_click.map(std::rc::Rc::new);
        let on_attach_click = self.on_attach_click.map(std::rc::Rc::new);
        let on_select_at_match = self.on_select_at_match.map(std::rc::Rc::new);
        let composer_bounds = self.composer_bounds.clone();
        let message_selection = self.message_selection;
        let on_select_message_char = self.on_select_message_char.clone();
        let on_drag_message_char = self.on_drag_message_char.clone();
        let expanded_thinkings = self.expanded_thinkings;

        let input_val = self.input_text.clone();
        let display_cwd = format_cwd_display(&self.cwd);
        let active_branch = self.git_branch.clone();
        let context_pct = self.context_pct;
        let active_mode = self.agent_mode.clone();

        // Text left edge = sidebar left + 8px scroll-area px + 4px outer-wrapper px.
        // Mouse events use window-absolute X, so we subtract this to get text-relative X.
        let win_w = window.viewport_size().width.to_f64() as f32;
        let sidebar_w = self.width;
        let msg_text_left = win_w - sidebar_w + 12.0;
        let streaming_text_left = win_w - sidebar_w + 8.0; // scroll-area px only (no inner px wrapper for streaming)

        div()
            .id("ai-sidebar-container")
            .flex()
            .flex_col()
            .w(px(self.width))
            .h_full()
            .bg(sidebar_bg)
            .border_l_1()
            .border_color(theme.border)
            .child(
                // 1. Header: Sleek, compact single toolbar (h: 36px) matching Fastty's design system
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                     .h(px(36.))
                     .w_full()
                     .px(px(12.))
                     // No visible divider under the header: section edges are imaginary.
                     // Left: Git branch
                    .child({
                        let has_branch = active_branch.is_some();
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .min_w(px(0.))
                            .flex_1()
                            .overflow_hidden()
                            .when_some(active_branch, |this, branch| {
                                this.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(4.))
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .child(crate::ui::icons::render_icon(
                                            IconType::GitBranch,
                                            theme.muted_strong,
                                            12.0,
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme.foreground)
                                                .truncate()
                                                .child(SharedString::from(branch)),
                                        ),
                                )
                            })
                            .when(!has_branch, |this| {
                                this.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(4.))
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .child(crate::ui::icons::render_icon(
                                            IconType::Folder,
                                            theme.muted,
                                            11.0,
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(theme.muted_strong)
                                                .truncate()
                                                .child(SharedString::from(display_cwd)),
                                        ),
                                )
                            })
                    })
                    // Right: Context indicator + Actions
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .flex_shrink_0()
                            // Context usage
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(3.5))
                                    .px(px(4.))
                                    .py(px(2.))
                                    .mr(px(2.))
                                    .child(render_context_ring(context_pct, &theme))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(theme.muted)
                                            .child(SharedString::from(format!("{:.0}%", context_pct * 100.0))),
                                    ),
                            )
                            // New Chat '+'
                            .child(
                                div()
                                    .id("ai-new-chat-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_color(theme.muted)
                                    .hover(move |s| s.bg(theme.hover).text_color(theme.foreground))
                                    .on_mouse_down(MouseButton::Left, {
                                        let on_new_chat = on_new_chat.clone();
                                        let on_clear = on_clear.clone();
                                        move |ev, window, cx| {
                                            if let Some(ref cb) = on_new_chat {
                                                cb(ev, window, cx);
                                            } else if let Some(ref cb) = on_clear {
                                                cb(ev, window, cx);
                                            }
                                        }
                                    })
                                    .child(crate::ui::icons::render_icon(
                                        IconType::Plus,
                                        theme.muted,
                                        12.0,
                                    )),
                            )
                            // Clear / History
                            .child(
                                div()
                                    .id("ai-clear-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_color(theme.muted)
                                    .hover(move |s| s.bg(theme.hover).text_color(theme.foreground))
                                    .on_mouse_down(MouseButton::Left, {
                                        let on_clear = on_clear.clone();
                                        move |ev, window, cx| {
                                            if let Some(ref cb) = on_clear {
                                                cb(ev, window, cx);
                                            }
                                        }
                                    })
                                    .child(crate::ui::icons::render_icon(
                                        IconType::RotateCcw,
                                        theme.muted,
                                        11.0,
                                    )),
                            )
                            // Close '✕'
                            .child(
                                div()
                                    .id("ai-close-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_color(theme.muted)
                                    .hover(move |s| s.bg(theme.hover).text_color(theme.foreground))
                                    .on_mouse_down(MouseButton::Left, {
                                        let on_close = on_close.clone();
                                        move |ev, window, cx| {
                                            if let Some(ref cb) = on_close {
                                                cb(ev, window, cx);
                                            }
                                        }
                                    })
                                    .child(crate::ui::icons::render_icon(
                                        IconType::X,
                                        theme.muted,
                                        11.0,
                                    )),
                            ),
                    ),
            )
            .child(
                // 2. Messages Scroll Area
                div()
                    .id("ai-messages-scroll-area")
                    .flex()
                    .flex_1()
                    .flex_col()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .py(px(8.))
                    .gap_4()
                    .when_some(self.scroll_handle.clone(), |this, sh| this.track_scroll(&sh))
                    // Container-level drag continuation: while a message
                    // selection drag is active, moves over gaps, code-block
                    // padding or list markers still update the selection via
                    // the nearest registered row.
                    .on_mouse_move({
                        let registry = row_registry.clone();
                        let on_drag = on_drag_message_char.clone();
                        move |ev: &MouseMoveEvent, window, cx| {
                            if ev.pressed_button != Some(MouseButton::Left) {
                                return;
                            }
                            let idx = crate::ui::markdown::row_at_point(&registry, ev.position);
                            let target = idx.and_then(|i| {
                                registry.borrow().get(i).map(|row| {
                                    (
                                        row.msg_idx,
                                        crate::ui::markdown::char_target_in_row(
                                            row,
                                            ev.position,
                                            window,
                                        ),
                                    )
                                })
                            });
                            if let (Some((msg_idx, target)), Some(ref cb)) =
                                (target, on_drag.as_ref())
                            {
                                cb(&(msg_idx, target), window, cx);
                            }
                        }
                    })
                    .children({
                        let sidebar_width = self.width;
                        let on_select_msg = on_select_message_char.clone();
                        let tool_pending_conf = pending_confirmation.clone();
                        let tool_on_confirm = on_confirm.clone();
                        let on_drag_msg = on_drag_message_char.clone();
                        let registry = row_registry.clone();
                        self.messages.clone().into_iter().enumerate().map(move |(msg_idx, msg)| {
                            if msg.is_user {
                                div()
                                    .w_full()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .items_end()
                                    .my(px(3.))
                                    .child(
                                        div()
                                            .max_w(px((sidebar_width - 32.0).max(120.0)))
                                            .rounded(px(10.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(theme.border)
                                            .px(px(12.))
                                            .py(px(8.))
                                            .cursor(CursorStyle::IBeam)
                                            // Fallback selection start: presses landing on the
                                            // bubble padding/border (where no text row is hit)
                                            // anchor at the nearest char of this message, so
                                            // right-to-left selections can start at the
                                            // question's end instead of doing nothing.
                                            .on_mouse_down(MouseButton::Left, {
                                                let registry = registry.clone();
                                                let on_select = on_select_msg.clone();
                                                move |ev: &MouseDownEvent, window, cx| {
                                                    let idx = crate::ui::markdown::row_at_point_in_message(
                                                        &registry, msg_idx, ev.position,
                                                    );
                                                    if let Some(i) = idx {
                                                        let target = registry.borrow().get(i).map(|row| {
                                                            crate::ui::markdown::char_target_in_row(
                                                                row, ev.position, window,
                                                            )
                                                        });
                                                        if let (Some(target), Some(ref cb)) =
                                                            (target, on_select.as_ref())
                                                        {
                                                            cb(&(msg_idx, target), window, cx);
                                                        }
                                                    }
                                                }
                                            })
                                            .when(!msg.images.is_empty(), |d| {
                                                d.child(
                                                    div()
                                                        .flex()
                                                        .flex_row()
                                                        .flex_wrap()
                                                        .gap_1()
                                                        .mb(px(6.))
                                                        .children(msg.images.iter().map(|img_path| {
                                                            let name = img_path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Image".to_string());
                                                            div()
                                                                .flex()
                                                                .flex_row()
                                                                .items_center()
                                                                .gap_1()
                                                                .px(px(6.))
                                                                .py(px(2.))
                                                                .rounded(px(4.))
                                                                .bg(theme.surface)
                                                                .border_1()
                                                                .border_color(theme.border)
                                                                .child(div().text_size(px(11.)).child("🖼"))
                                                                .child(
                                                                    div()
                                                                        .text_size(px(11.))
                                                                        .text_color(theme.foreground)
                                                                        .max_w(px(140.))
                                                                        .overflow_hidden()
                                                                        .child(SharedString::from(name)),
                                                                )
                                                        })),
                                                )
                                            })
                                            .when(!msg.documents.is_empty(), |d| {
                                                d.child(
                                                    div()
                                                        .flex()
                                                        .flex_row()
                                                        .flex_wrap()
                                                        .gap_1()
                                                        .mb(px(6.))
                                                        .children(msg.documents.iter().map(|doc_path| {
                                                            let name = doc_path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Document.pdf".to_string());
                                                            div()
                                                                .flex()
                                                                .flex_row()
                                                                .items_center()
                                                                .gap_1()
                                                                .px(px(6.))
                                                                .py(px(2.))
                                                                .rounded(px(4.))
                                                                .bg(theme.surface)
                                                                .border_1()
                                                                .border_color(theme.border)
                                                                .child(div().text_size(px(11.)).child("📄"))
                                                                .child(
                                                                    div()
                                                                        .text_size(px(11.))
                                                                        .text_color(theme.foreground)
                                                                        .max_w(px(140.))
                                                                        .overflow_hidden()
                                                                        .child(SharedString::from(name)),
                                                                )
                                                        })),
                                                )
                                            })
                                            .when(!msg.text.is_empty(), |d| {
                                                let active_sel = if let Some((sel_msg, s, e)) = message_selection {
                                                    if sel_msg == msg_idx { Some((s, e)) } else { None }
                                                } else {
                                                    None
                                                };
                                                let on_select_cb = on_select_msg.as_ref().map(|cb| {
                                                    let cb = cb.clone();
                                                    std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                                        cb(&(msg_idx, target), window, cx);
                                                    }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                                                });
                                                let on_drag_cb = on_drag_msg.as_ref().map(|cb| {
                                                    let cb = cb.clone();
                                                    std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                                        cb(&(msg_idx, target), window, cx);
                                                    }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                                                });
                                                // Scroll area px(8) + bubble right-align offset when at max_w
                                                // + bubble inner px(12) left padding = 44px from sidebar left.
                                                let user_text_left = (win_w - sidebar_width + 44.0).max(0.0);
                                                d.child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .text_color(theme.foreground)
                                                        .w_full()
                                                        .min_w(px(0.))
                                                        .child(render_markdown_prose(
                                                            &msg.text,
                                                            &theme,
                                                            false,
                                                            active_sel,
                                                            on_select_cb,
                                                            on_drag_cb,
                                                            user_text_left,
                                                            Some(win_ref),
                                                            Some((&registry, msg_idx)),
                                                        )),
                                                )
                                            }),
                                    )
                            } else {
                                div()
                                    .flex()
                                    .flex_col()
                                    .w_full()
                                    .min_w(px(0.))
                                    .px(px(4.))
                                    .my(px(3.))
                                    .gap_3()
                                // Collapsible reasoning line
                                .when_some(msg.thinking.clone(), |this, thinking| {
                                    let summary_snip: String = thinking.lines().next().unwrap_or("Model reasoning").chars().take(48).collect();
                                    let is_thinking_expanded = expanded_thinkings.contains(&msg_idx);
                                    let on_toggle_thinking = on_toggle_thinking.clone();
                                    this.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w_full()
                                            .gap_1()
                                            .my(px(1.))
                                            .child(
                                                div()
                                                    .id(("ai-thinking-header", msg_idx))
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1()
                                                    .py(px(2.))
                                                    .pr(px(6.))
                                                    .rounded(px(4.))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .hover(move |s| s.bg(theme.hover))
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let on_toggle_thinking = on_toggle_thinking.clone();
                                                        move |_ev, window, cx| {
                                                            if let Some(ref cb) = on_toggle_thinking {
                                                                cb(&msg_idx, window, cx);
                                                            }
                                                        }
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child(if is_thinking_expanded { "⌵" } else { "›" }),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .text_color(theme.muted)
                                                            .overflow_hidden()
                                                            .child(SharedString::from(format!(
                                                                "Thought · {}...",
                                                                summary_snip
                                                            ))),
                                                    ),
                                            )
                                            .when(is_thinking_expanded, |d| {
                                                d.child(
                                                    div()
                                                        .w_full()
                                                        .min_w(px(0.))
                                                        .pl(px(14.))
                                                        .text_size(px(11.5))
                                                        .text_color(theme.muted_strong)
                                                        .overflow_hidden()
                                                        .child(thinking),
                                                )
                                            }),
                                    )
                                })
                                // Tool calls: distinct cards with full padding and margins
                                .when(!msg.tool_calls.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w_full()
                                            .gap_2()
                                            .my(px(2.))
                                              .children(msg.tool_calls.into_iter().enumerate().map(|(tc_idx, tc)| {
                                                 let info = parse_tool_row(&tc);
                                                 let is_edit = tc.name == "edit_file";
                                                 let awaiting = tool_pending_conf
                                                     .as_ref()
                                                     .filter(|p| p.tool_id == tc.id)
                                                     .cloned();
                                                 // Card source: the applied diff once the tool
                                                 // finishes; while running/pending a preview built
                                                 // from the raw args, so the diff is visible
                                                 // immediately — never a bare spinner.
                                                 let (card_diff, card_running) =
                                                     if let Some(diff) = tc.diff.clone() {
                                                         (diff, false)
                                                     } else if is_edit {
                                                         match parse_diff_confirmation("edit_file", &tc.args) {
                                                             Some(p) => (
                                                                 crate::ai::diff::FileDiff {
                                                                     path: p.file_path,
                                                                     lines: p.rows.into_iter().map(|r| {
                                                                         let (kind, text) = match r {
                                                                             DiffRow::Context(t) => (crate::ai::diff::DiffLineKind::Context, t),
                                                                             DiffRow::Del(t) => (crate::ai::diff::DiffLineKind::Del, t),
                                                                             DiffRow::Add(t) => (crate::ai::diff::DiffLineKind::Add, t),
                                                                         };
                                                                         crate::ai::diff::DiffLine {
                                                                             kind,
                                                                             text,
                                                                             old_line: None,
                                                                             new_line: None,
                                                                         }
                                                                     }).collect(),
                                                                     added: p.add_count,
                                                                     removed: p.del_count,
                                                                     truncated: false,
                                                                 },
                                                                 tc.is_running,
                                                             ),
                                                             None => (
                                                                 crate::ai::diff::FileDiff {
                                                                     path: info.target.clone(),
                                                                     lines: Vec::new(),
                                                                     added: 0,
                                                                     removed: 0,
                                                                     truncated: false,
                                                                 },
                                                                 tc.is_running,
                                                             ),
                                                         }
                                                     } else {
                                                         (
                                                             crate::ai::diff::FileDiff {
                                                                 path: String::new(),
                                                                 lines: Vec::new(),
                                                                 added: 0,
                                                                 removed: 0,
                                                                 truncated: false,
                                                             },
                                                             false,
                                                         )
                                                     };
                                                 let show_card = is_edit || tc.diff.is_some();
                                                 div()
                                                     .flex()
                                                     .flex_col()
                                                     .w_full()
                                                     .gap_1()
                                                     .child(
                                                     div()
                                                     .flex()
                                                     .flex_row()
                                                     .items_center()
                                                     .justify_between()
                                                     .px(px(10.))
                                                     .py(px(6.))
                                                     .rounded(px(6.))
                                                     .bg(theme.surface)
                                                     .border_1()
                                                     .border_color(theme.border)
                                                     .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .flex_1()
                                                            .min_w(px(0.))
                                                            .overflow_hidden()
                                                            .child(crate::ui::icons::render_icon(
                                                                info.icon_type,
                                                                theme.muted,
                                                                12.0,
                                                            ))
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .text_color(theme.muted_strong)
                                                                    .flex_shrink_0()
                                                                    .child(info.action),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .text_color(theme.foreground)
                                                                    .overflow_hidden()
                                                                    .child(SharedString::from(info.target)),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .ml(px(8.))
                                                            .child(if info.is_running {
                                                                crate::ui::icons::render_spinner(theme.accent, 14.0, ElementId::named_usize("tool-spinner", tc_idx)).into_any_element()
                                                            } else {
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_1()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(11.))
                                                                            .text_color(if info.is_error {
                                                                                theme.bright_red
                                                                            } else {
                                                                                theme.muted
                                                                            })
                                                                            .child(SharedString::from(info.status_text)),
                                                                    )
                                                                    .child(crate::ui::icons::render_icon(
                                                                        if info.is_error {
                                                                            IconType::X
                                                                        } else {
                                                                            IconType::Check
                                                                        },
                                                                        if info.is_error {
                                                                            theme.bright_red
                                                                        } else {
                                                                            theme.green
                                                                        },
                                                                        11.0,
                                                                    ))
                                                                     .into_any_element()
                                                             })
                                                     )
                                                     )
                                                     .when(show_card, |d| {
                                                         d.child(render_tool_diff_card(
                                                             &theme,
                                                             &card_diff,
                                                             awaiting.is_some(),
                                                             card_running,
                                                             awaiting.as_ref().map(|_| tool_on_confirm.clone()).flatten(),
                                                             tc_idx,
                                                         ))
                                                     })
                                              })),
                                    )
                                })
                                // Assistant prose
                                .when(!msg.text.is_empty(), |this| {
                                    let text_to_copy = msg.text.clone();
                                    let active_sel = if let Some((sel_msg, s, e)) = message_selection {
                                        if sel_msg == msg_idx { Some((s, e)) } else { None }
                                    } else {
                                        None
                                    };

                                    let on_select_cb = on_select_msg.as_ref().map(|cb| {
                                        let cb = cb.clone();
                                        std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                            cb(&(msg_idx, target), window, cx);
                                        }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                                    });

                                    let on_drag_cb = on_drag_msg.as_ref().map(|cb| {
                                        let cb = cb.clone();
                                        std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                            cb(&(msg_idx, target), window, cx);
                                        }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                                    });

                                    this.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w_full()
                                            .min_w(px(0.))
                                            .gap_1()
                                            .on_mouse_down(MouseButton::Left, {
                                                let registry = registry.clone();
                                                let on_select = on_select_cb.clone();
                                                move |ev: &MouseDownEvent, window, cx| {
                                                    let idx = crate::ui::markdown::row_at_point_in_message(
                                                        &registry, msg_idx, ev.position,
                                                    );
                                                    if let Some(i) = idx {
                                                        let target = registry.borrow().get(i).map(|row| {
                                                            crate::ui::markdown::char_target_in_row(
                                                                row, ev.position, window,
                                                            )
                                                        });
                                                        if let (Some(target), Some(ref cb)) =
                                                            (target, on_select.as_ref())
                                                        {
                                                            cb(target, window, cx);
                                                        }
                                                    }
                                                }
                                            })
                                            .child(
                                                div()
                                                    .w_full()
                                                    .min_w(px(0.))
                                                    .overflow_hidden()
                                                    .cursor(CursorStyle::IBeam)
                                                    .child(render_markdown_prose(
                                                        &msg.text,
                                                        &theme,
                                                        false,
                                                        active_sel,
                                                        on_select_cb,
                                                        on_drag_cb,
                                                        msg_text_left,
                                                        Some(win_ref),
                                                        Some((&registry, msg_idx)),
                                                    ))
                                            )
                                             .child(
                                                 div()
                                                     .flex()
                                                     .flex_row()
                                                     .justify_end()
                                                     .items_center()
                                                     .pt(px(2.))
                                                     .child({
                                                         let on_copied = on_copied.clone();
                                                         div()
                                                             .flex()
                                                             .flex_row()
                                                             .items_center()
                                                             .gap_1()
                                                             .px(px(6.))
                                                             .py(px(2.))
                                                             .rounded(px(4.))
                                                             .cursor(CursorStyle::PointingHand)
                                                             .text_size(px(11.))
                                                             .text_color(if copy_feedback { theme.green } else { theme.muted })
                                                             .hover(move |s| s.bg(theme.surface_raised).text_color(theme.foreground))
                                                             .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                                                 if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                                                     let _ = clip.set_text(text_to_copy.clone());
                                                                 }
                                                                 if let Some(ref cb) = on_copied {
                                                                     cb(ev, window, cx);
                                                                 }
                                                             })
                                                             .child(crate::ui::icons::render_icon(if copy_feedback { IconType::Check } else { IconType::Copy }, if copy_feedback { theme.green } else { theme.muted }, 11.0))
                                                             .child(if copy_feedback { "Copied" } else { "Copy" })
                                                     })
                                             )
                                    )
                                })
                        }
                    })
                })
                    // 3. Streaming Activity
                    .when(self.is_streaming, |this| {
                        let stream_idx = self.messages.len();
                        let registry = row_registry.clone();
                        let active_sel = if let Some((sel_msg, s, e)) = message_selection {
                            if sel_msg == stream_idx { Some((s, e)) } else { None }
                        } else {
                            None
                        };
                        let on_select_cb = on_select_message_char.as_ref().map(|cb| {
                            let cb = cb.clone();
                            std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                cb(&(stream_idx, target), window, cx);
                            }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                        });
                        let on_drag_cb = on_drag_message_char.as_ref().map(|cb| {
                            let cb = cb.clone();
                            std::sync::Arc::new(move |target: usize, window: &mut Window, cx: &mut App| {
                                cb(&(stream_idx, target), window, cx);
                            }) as std::sync::Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>
                        });

                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_start()
                                .gap_2()
                                .w_full()
                                .when(!self.streaming_thinking.is_empty(), |d| {
                                    let thinking_snip: String = self.streaming_thinking.lines().next().unwrap_or("Thinking...").chars().take(48).collect();
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .w_full()
                                            .min_w(px(0.))
                                            .overflow_hidden()
                                            .text_size(px(11.5))
                                            .text_color(theme.muted)
                                            .child(crate::ui::icons::render_icon(
                                                IconType::Sparkles,
                                                theme.muted,
                                                12.0,
                                            ))
                                            .child(SharedString::from(format!("Thinking · {}...", thinking_snip)))
                                            .with_animation(
                                                "ai-reasoning-pulse",
                                                Animation::new(Duration::from_secs(2))
                                                    .repeat()
                                                    .with_easing(pulsating_between(0.55, 1.0))
                                                    .with_max_fps(30.),
                                                |el, delta| el.opacity(delta),
                                            ),
                                    )
                                })
                                .when(self.streaming_text.is_empty(), |d| {
                                    d.child(render_thinking_indicator(&theme))
                                })
                                .when(!self.streaming_text.is_empty(), |d| {
                                    d.child(render_markdown_prose(
                                        &self.streaming_text,
                                        &theme,
                                        true,
                                        active_sel,
                                        on_select_cb,
                                        on_drag_cb,
                                        streaming_text_left,
                                        Some(win_ref),
                                        Some((&registry, stream_idx)),
                                    ))
                                }),
                        )
                    })
                    // 4. Pending Permission Confirmation (Diff Card or Command Prompt)
                    .when_some(self.pending_confirmation.clone(), |this, conf| {
                        this.child(render_confirmation_section(&theme, &conf, on_confirm.clone()))
                    }),
            )
            .child(
                // 5. Inset Composer Card & Footer
                // No visible divider above the composer: section edges are imaginary.
                div()
                    .flex()
                    .flex_col()
                    .p(px(12.))
                    .pt(px(6.))
                    .child(
                        // Composer Inset Box
                        div()
                            .id("ai-input-box")
                            .flex()
                            .flex_col()
                            .p(px(10.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(if is_focused { theme.accent } else { theme.border })
                            .bg(surface_raised)
                            .cursor(CursorStyle::IBeam)
                            .on_mouse_down(MouseButton::Left, {
                                let on_focus = on_focus.clone();
                                move |ev, window, cx| {
                                    if let Some(ref cb) = on_focus {
                                        cb(ev, window, cx);
                                    }
                                }
                            })
                            // @ Mention Popup
                            .when(self.at_menu_open && !self.at_matches.is_empty(), |d| {
                                let on_select_at_match = on_select_at_match.clone();
                                d.child(
                                    div()
                                        .id("ai-at-mention-popup")
                                        .w_full()
                                        .max_h(px(160.))
                                        .overflow_y_scroll()
                                        .rounded(px(6.))
                                        .bg(theme.surface)
                                        .border_1()
                                        .border_color(theme.border)
                                        .mb(px(8.))
                                        .p(px(4.))
                                        .children(self.at_matches.iter().take(20).map(|file_path| {
                                            let fp = file_path.clone();
                                            let on_select = on_select_at_match.clone();
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2()
                                                .px(px(8.))
                                                .py(px(4.))
                                                .rounded(px(4.))
                                                .min_w(px(0.))
                                                .overflow_hidden()
                                                .cursor(CursorStyle::PointingHand)
                                                .hover(move |s| s.bg(theme.hover))
                                                .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                                    if let Some(ref cb) = on_select {
                                                        cb(&fp, window, cx);
                                                    }
                                                })
                                                .child(crate::ui::icons::render_icon(IconType::FileCode, theme.muted, 12.0))
                                                .child(
                                                    div()
                                                        .text_size(px(12.))
                                                        .text_color(theme.foreground)
                                                        .overflow_hidden()
                                                        .child(SharedString::from(file_path.clone())),
                                                )
                                        })),
                                )
                            })
                            // Attached Files Chip Badges
                            .when(!self.attached_files.is_empty(), |d| {
                                let on_remove_attachment = on_remove_attachment.clone();
                                d.child(
                                    div()
                                        .id("ai-attached-files-row")
                                        .flex()
                                        .flex_row()
                                        .flex_wrap()
                                        .gap_1()
                                        .pb(px(6.))
                                        .children(self.attached_files.iter().enumerate().map(|(idx, path)| {
                                            let is_img = is_image_path(path);
                                            let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
                                            let on_remove = on_remove_attachment.clone();
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .child(if is_img { "🖼" } else { "📄" }),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(theme.foreground)
                                                        .max_w(px(120.))
                                                        .overflow_hidden()
                                                        .child(SharedString::from(name)),
                                                )
                                                .child(
                                                    div()
                                                        .ml(px(2.))
                                                        .text_size(px(10.))
                                                        .text_color(theme.muted)
                                                        .cursor(CursorStyle::PointingHand)
                                                        .hover(move |s| s.text_color(theme.bright_red))
                                                        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                                            if let Some(ref cb) = on_remove {
                                                                cb(&idx, window, cx);
                                                            }
                                                        })
                                                        .child("✕"),
                                                )
                                        })),
                                )
                            })
                            // Text Input Area
                            .child(
                                div()
                                    .id("ai-composer-input-scroll")
                                    .relative()
                                    .child({
                                        let bounds_cell = composer_bounds.clone();
                                        canvas(
                                            |_, _, _| {},
                                            move |bounds, _, _, _| {
                                                if let Some(ref c) = bounds_cell {
                                                    c.set(Some(bounds));
                                                }
                                            },
                                        )
                                        .absolute()
                                        .size_full()
                                    })
                                    .w_full()
                                    .min_h(px(44.))
                                    .max_h(px(140.))
                                    .overflow_y_scroll()
                                    .child(if input_val.is_empty() {
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .when(is_focused, |this| {
                                                this.child(
                                                    div()
                                                        .w(px(2.))
                                                        .h(px(14.))
                                                        .rounded(px(1.))
                                                        .bg(theme.accent)
                                                        .mr(px(2.))
                                                        .flex_shrink_0(),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_size(px(12.5))
                                                    .text_color(theme.muted)
                                                    .child(if active_mode == "Ask" {
                                                        "Ask a question about the code..."
                                                    } else {
                                                        "Describe the task, or type @ to reference a file"
                                                    }),
                                            )
                                    } else {
                                        let avail_w = (self.width - 44.0).max(80.0);
                                        let max_cols = ((avail_w / 7.2).floor() as usize).max(10);
                                        let lines = crate::ui::text_input::wrap_text_into_lines(&input_val, max_cols);
                                        let num_lines = lines.len();
                                        let cursor_pos = self.cursor_pos;
                                        let selection = self.selection;
                                        let sidebar_w = self.width;

                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .children(lines.into_iter().enumerate().map(|(line_idx, line)| {
                                                let is_last = line_idx + 1 == num_lines;
                                                let line_len = line.text.chars().count();
                                                let line_end = line.start_char + line_len;
                                                let is_cursor_on_line = is_focused && (
                                                    (cursor_pos >= line.start_char && cursor_pos < line_end)
                                                    || (is_last && cursor_pos >= line.start_char && cursor_pos <= line_end)
                                                );
                                                let cursor_arg = if is_cursor_on_line { Some(cursor_pos) } else { None };

                                                let on_click_char = on_click_char.clone();
                                                let line_start = line.start_char;
                                                let line_text_clone = line.text.clone();
                                                let composer_bounds = composer_bounds.clone();

                                                div()
                                                    .w_full()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .cursor(CursorStyle::IBeam)
                                                    .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                                        let click_x = ev.position.x.to_f64() as f32;
                                                        let rel_x = if let Some(ref cbounds) = composer_bounds {
                                                            if let Some(b) = cbounds.get() {
                                                                (click_x - b.origin.x.to_f64() as f32).max(0.0)
                                                            } else {
                                                                let win_w = window.viewport_size().width.to_f64() as f32;
                                                                (click_x - (win_w - sidebar_w + 24.0)).max(0.0)
                                                            }
                                                        } else {
                                                            let win_w = window.viewport_size().width.to_f64() as f32;
                                                            (click_x - (win_w - sidebar_w + 24.0)).max(0.0)
                                                        };

                                                        let col = crate::ui::text_input::index_for_x(&line_text_clone, rel_x, 13.0, window);
                                                        let target = line_start + col.min(line_len);
                                                        if let Some(ref cb) = on_click_char {
                                                            cb(&target, window, cx);
                                                        }
                                                    })
                                                    .child(crate::ui::text_input::render_line_spans(
                                                        &line.text,
                                                        line.start_char,
                                                        cursor_arg,
                                                        selection,
                                                        13.0,
                                                        &theme,
                                                        Some(win_ref),
                                                    ))
                                            }))
                                    }),
                            )
                            // Inset Toolbar: Mode Selector + Action Buttons + Send/Stop
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .w_full()
                                    .pt(px(8.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            // Mode Segmented Pill [ Agente | Preguntar ]
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .rounded(px(6.))
                                                    .bg(sidebar_bg)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .p(px(2.))
                                                    .child(
                                                        div()
                                                            .px(px(8.))
                                                            .py(px(3.))
                                                            .rounded(px(4.))
                                                            .bg(if active_mode == "Agent" {
                                                                surface_raised
                                                            } else {
                                                                gpui::transparent_black()
                                                            })
                                                            .text_size(px(11.))
                                                            .font_weight(if active_mode == "Agent" {
                                                                FontWeight::BOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .text_color(if active_mode == "Agent" {
                                                                theme.foreground
                                                            } else {
                                                                theme.muted
                                                            })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, {
                                                                let on_toggle_mode = on_toggle_mode.clone();
                                                                move |_ev, window, cx| {
                                                                    if let Some(ref cb) = on_toggle_mode {
                                                                        cb(&"Agent".to_string(), window, cx);
                                                                    }
                                                                }
                                                            })
                                                            .child("Agent"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px(px(8.))
                                                            .py(px(3.))
                                                            .rounded(px(4.))
                                                            .bg(if active_mode == "Ask" {
                                                                surface_raised
                                                            } else {
                                                                gpui::transparent_black()
                                                            })
                                                            .text_size(px(11.))
                                                            .font_weight(if active_mode == "Ask" {
                                                                FontWeight::BOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .text_color(if active_mode == "Ask" {
                                                                theme.foreground
                                                            } else {
                                                                theme.muted
                                                            })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, {
                                                                let on_toggle_mode = on_toggle_mode.clone();
                                                                move |_ev, window, cx| {
                                                                    if let Some(ref cb) = on_toggle_mode {
                                                                        cb(&"Ask".to_string(), window, cx);
                                                                    }
                                                                }
                                                            })
                                                            .child("Ask"),
                                                    ),
                                            )
                                            // Action Buttons: @ and Paperclip (Vector line icons)
                                            .child(
                                                div()
                                                    .id("ai-at-mention-btn")
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .w(px(24.))
                                                    .h(px(24.))
                                                    .rounded(px(4.))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .hover(move |s| s.bg(theme.hover))
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let on_at_click = on_at_click.clone();
                                                        move |ev, window, cx| {
                                                            if let Some(ref cb) = on_at_click {
                                                                cb(ev, window, cx);
                                                            }
                                                        }
                                                    })
                                                    .child(crate::ui::icons::render_at_sign_icon(theme.muted, 14.0)),
                                            )
                                            .child(
                                                div()
                                                    .id("ai-attachment-btn")
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .w(px(24.))
                                                    .h(px(24.))
                                                    .rounded(px(4.))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .hover(move |s| s.bg(theme.hover))
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let on_attach_click = on_attach_click.clone();
                                                        move |ev, window, cx| {
                                                            if let Some(ref cb) = on_attach_click {
                                                                cb(ev, window, cx);
                                                            }
                                                        }
                                                    })
                                                    .child(crate::ui::icons::render_paperclip_icon(theme.muted, 14.0)),
                                            ),
                                    )
                                    .child(
                                        if self.is_streaming {
                                            div()
                                                .id("ai-stop-btn")
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .px(px(10.))
                                                .py(px(4.))
                                                .rounded(px(6.))
                                                .bg(surface_raised)
                                                .border_1()
                                                .border_color(theme.border)
                                                .cursor(CursorStyle::PointingHand)
                                                .hover(move |s| s.bg(theme.hover))
                                                .child(
                                                    div()
                                                        .w(px(8.))
                                                        .h(px(8.))
                                                        .bg(theme.foreground)
                                                        .rounded(px(1.)),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.5))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(theme.foreground)
                                                        .child("Stop"),
                                                )
                                                .on_mouse_down(MouseButton::Left, {
                                                    let on_cancel = on_cancel.clone();
                                                    move |ev, window, cx| {
                                                        if let Some(ref cb) = on_cancel {
                                                            cb(ev, window, cx);
                                                        }
                                                    }
                                                })
                                        } else {
                                            let is_empty = input_val.trim().is_empty() && self.attached_files.is_empty();
                                            div()
                                                .id("ai-submit-btn")
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .px(px(10.))
                                                .py(px(4.))
                                                .rounded(px(6.))
                                                .bg(if is_empty {
                                                    sidebar_bg
                                                } else {
                                                    surface_raised
                                                })
                                                .border_1()
                                                .border_color(theme.border)
                                                .cursor(if is_empty {
                                                    CursorStyle::Arrow
                                                } else {
                                                    CursorStyle::PointingHand
                                                })
                                                .hover(move |s| {
                                                    if !is_empty {
                                                        s.bg(theme.hover).border_color(theme.accent)
                                                    } else {
                                                        s
                                                    }
                                                })
                                                .child(
                                                    div()
                                                        .text_size(px(11.5))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(if is_empty {
                                                            theme.muted
                                                        } else {
                                                            theme.foreground
                                                        })
                                                        .child("Send ↑"),
                                                )
                                                .on_mouse_down(MouseButton::Left, {
                                                    let on_submit = on_submit.clone();
                                                    move |ev, window, cx| {
                                                        if let Some(ref cb) = on_submit {
                                                            cb(ev, window, cx);
                                                        }
                                                    }
                                                })
                                        },
                                    ),
                            ),
                    )
            )
    }
}
