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
}

#[derive(Debug, Clone)]
pub struct AiUiMessage {
    pub is_user: bool,
    pub text: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<AiUiToolCall>,
    pub timestamp: Option<String>,
}

impl AiUiMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            is_user: true,
            text: text.into(),
            thinking: None,
            tool_calls: Vec::new(),
            timestamp: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            is_user: false,
            text: text.into(),
            thinking: None,
            tool_calls: Vec::new(),
            timestamp: None,
        }
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
        }
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
        "failed".to_string()
    } else if let Some(ref out) = tc.output {
        if tc.name == "run_command" {
            if let Some(pos) = out.find("failed") {
                let start = out[..pos]
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace() || *c == ':')
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                let end = pos + "failed".len();
                let snip = out.get(start..end).map(|s| s.trim()).unwrap_or("");
                if !snip.is_empty() {
                    snip.to_string()
                } else {
                    "done".to_string()
                }
            } else if out.contains("test result: ok") {
                "ok".to_string()
            } else {
                "done".to_string()
            }
        } else if tc.name == "search" || tc.name == "list_dir" {
            let lines = out.lines().count();
            if lines > 0 {
                format!("{} matches", lines)
            } else {
                "done".to_string()
            }
        } else if tc.name == "read_file" {
            let lines = out.lines().count();
            if lines > 0 {
                format!("L1-{}", lines)
            } else {
                "done".to_string()
            }
        } else if tc.name == "edit_file" {
            "+1 -1".to_string()
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
const DIFF_MAX_ROWS: usize = 24;

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

    if let Some(diff) = parse_diff_confirmation(&conf.tool_name, &conf.input_summary) {
        let on_confirm_allow = on_confirm.clone();
        let on_confirm_always = on_confirm.clone();
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
            // Diff body
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(10.))
                    .bg(theme.main_bg)
                    .gap(px(2.))
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
                            .when_some(tint, |d, tint| d.bg(tint))
                            .child(SharedString::from(format!("{}{}", prefix, text)))
                    })),
            )
            // Actions
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
        let suffix = old_lines[prefix..]
            .iter()
            .zip(new_lines[prefix..].iter())
            .rev()
            .take_while(|(a, b)| a == b)
            .count();
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

fn render_stream_cursor(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .ml(px(4.))
        .w(px(2.))
        .h(px(13.))
        .rounded(px(1.))
        .bg(theme.accent)
        .with_animation(
            "ai-stream-cursor",
            Animation::new(Duration::from_millis(1100))
                .repeat()
                .with_easing(pulsating_between(0.25, 1.0))
                .with_max_fps(30.),
            |el, delta| el.opacity(delta),
        )
}

fn render_markdown_prose(text: &str, theme: &Theme, is_streaming: bool) -> Div {
    let mut container = div().flex().flex_col().gap_2().w_full().min_w(px(0.));
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let num_paras = paragraphs.len();

    for (p_idx, para) in paragraphs.into_iter().enumerate() {
        let is_last_para = p_idx + 1 == num_paras;

        if !para.contains('`') {
            let mut para_div = div()
                .w_full()
                .min_w(px(0.))
                .text_size(px(13.))
                .text_color(theme.foreground)
                .child(para.to_string());
            if is_last_para && is_streaming {
                para_div = para_div.child(render_stream_cursor(theme));
            }
            container = container.child(para_div);
            continue;
        }

        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1()
            .w_full()
            .min_w(px(0.));

        let parts: Vec<&str> = para.split('`').collect();
        for (i, part) in parts.into_iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i % 2 == 1 {
                row = row.child(
                    div()
                        .px(px(5.))
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(theme.surface_raised)
                        .border_1()
                        .border_color(theme.border)
                        .text_size(px(12.))
                        .text_color(theme.accent)
                        .font_weight(FontWeight::MEDIUM)
                        .child(part.to_string()),
                );
            } else {
                row = row.child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.foreground)
                        .child(part.to_string()),
                );
            }
        }

        if is_last_para && is_streaming {
            row = row.child(
                div()
                    .w(px(2.))
                    .h(px(14.))
                    .rounded(px(1.))
                    .bg(theme.accent)
                    .flex_shrink_0()
                    .with_animation(
                        "ai-stream-cursor-inline",
                        Animation::new(Duration::from_millis(1100))
                            .repeat()
                            .with_easing(pulsating_between(0.25, 1.0))
                            .with_max_fps(30.),
                        |el, delta| el.opacity(delta),
                    ),
            );
        }

        container = container.child(row);
    }

    container
}

impl RenderOnce for AiSidebar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let sidebar_bg = Hsla { a: theme.opacity, ..theme.sidebar_bg };
        let surface_raised = theme.surface_raised;

        let is_focused = self.is_focused;
        let on_close = self.on_close.map(std::rc::Rc::new);
        let on_clear = self.on_clear.map(std::rc::Rc::new);
        let on_cancel = self.on_cancel.map(std::rc::Rc::new);
        let on_submit = self.on_submit.map(std::rc::Rc::new);
        let on_confirm = self.on_confirm.map(std::rc::Rc::new);
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
        let expanded_thinkings = self.expanded_thinkings;

        let input_val = self.input_text.clone();
        let display_cwd = format_cwd_display(&self.cwd);
        let active_branch = self.git_branch.clone();
        let context_pct = self.context_pct;
        let active_mode = self.agent_mode.clone();

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
                // 1. Header: identity · context · actions on one row
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .px(px(14.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .child("Fastty AI"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .text_size(px(11.))
                                            .text_color(theme.muted)
                                            .overflow_hidden()
                                            .child(SharedString::from(display_cwd))
                                            .when_some(active_branch, |this, branch| {
                                                this.child(SharedString::from(format!("· {}", branch)))
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            // Context usage
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .mr(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.muted)
                                            .child(SharedString::from(format!("{:.0}%", context_pct * 100.0))),
                                    )
                                    .child(render_context_ring(context_pct, &theme)),
                            )
                            // New Chat '+'
                            .child(
                                div()
                                    .id("ai-new-chat-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(24.))
                                    .h(px(24.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(theme.hover))
                                    .text_color(theme.muted)
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
                                        13.0,
                                    )),
                            )
                            // History / Clear
                            .child(
                                div()
                                    .id("ai-clear-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(24.))
                                    .h(px(24.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(theme.hover))
                                    .text_color(theme.muted)
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
                                        12.0,
                                    )),
                            )
                            // Close '✕'
                            .child(
                                div()
                                    .id("ai-close-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(px(24.))
                                    .h(px(24.))
                                    .rounded(px(4.))
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(theme.hover))
                                    .text_color(theme.muted)
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
                                        12.0,
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
                    .gap_3()
                    .children(self.messages.into_iter().enumerate().map(|(msg_idx, msg)| {
                        if msg.is_user {
                            // User message: plain text, no bubble
                            div()
                                .w_full()
                                .px(px(4.))
                                .text_size(px(13.))
                                .text_color(theme.foreground)
                                .child(msg.text.clone())
                        } else {
                            div()
                                .flex()
                                .flex_col()
                                .w_full()
                                .gap_2()
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
                                                        .pl(px(14.))
                                                        .text_size(px(11.5))
                                                        .text_color(theme.muted_strong)
                                                        .child(thinking),
                                                )
                                            }),
                                    )
                                })
                                // Tool calls: flat rows with a left hairline
                                .when(!msg.tool_calls.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w_full()
                                            .border_l_1()
                                            .border_color(theme.border)
                                            .pl(px(10.))
                                            .children(msg.tool_calls.into_iter().enumerate().map(|(tc_idx, tc)| {
                                                let info = parse_tool_row(&tc);
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .py(px(4.))
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
                                            })),
                                    )
                                })
                                // Assistant prose
                                .when(!msg.text.is_empty(), |this| {
                                    this.child(render_markdown_prose(&msg.text, &theme, false))
                                })
                        }
                    }))
                    // 3. Streaming Activity
                    .when(self.is_streaming, |this| {
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
                                    d.child(render_markdown_prose(&self.streaming_text, &theme, true))
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
                div()
                    .flex()
                    .flex_col()
                    .p(px(12.))
                    .pt(px(6.))
                    .border_t_1()
                    .border_color(theme.border)
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
                                        let avail_w = (self.width - 48.0).max(80.0);
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

                                                div()
                                                    .w_full()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .cursor(CursorStyle::IBeam)
                                                    .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                                        let click_x = ev.position.x.to_f64() as f32;
                                                        let container_left = (sidebar_w - avail_w).max(0.0) / 2.0;
                                                        let rel_x = (click_x - container_left).max(0.0);
                                                        let col = (rel_x / 7.2).round() as usize;
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
                                            let is_empty = input_val.trim().is_empty();
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
