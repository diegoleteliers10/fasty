//! Time-of-day widget. Hand-rolled formatter — no `chrono` dependency.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::widgets::{Align, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 1000;

pub struct TimeWidget {
    format: String,
    align: Align,
    last_poll: Instant,
    interval: Duration,
    cached: String,
    timezone_offset_hours: Option<i32>,
}

impl TimeWidget {
    pub fn new(format: String, align: Align, interval_ms: Option<u64>, timezone: Option<i32>) -> Self {
        Self {
            format,
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            cached: String::new(),
            timezone_offset_hours: timezone,
        }
    }
}

impl Widget for TimeWidget {
    fn id(&self) -> &'static str { "time" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration { self.interval }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, _ctx: &WidgetContext) {
        self.cached = format_now(&self.format, self.timezone_offset_hours);
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        if self.cached.is_empty() {
            return Vec::new();
        }
        vec![Segment {
            text: format!(" {} ", self.cached),
            color: [0.80, 0.82, 0.88, 1.0],
            tooltip: Some("local time".to_string()),
        }]
    }
}

fn format_now(fmt: &str, timezone_offset_hours: Option<i32>) -> String {
    let mut secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let offset_secs = if let Some(h) = timezone_offset_hours {
        h as i64 * 3600
    } else {
        get_local_time_offset_secs() as i64
    };

    secs += offset_secs;
    if secs < 0 {
        secs = 0;
    }

    let (h, m, s) = (
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60,
    );
    let mut out = String::with_capacity(fmt.len());
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('H') => out.push_str(&format!("{:02}", h)),
                Some('M') => out.push_str(&format!("{:02}", m)),
                Some('S') => out.push_str(&format!("{:02}", s)),
                Some('%') => out.push('%'),
                Some(other) => { out.push('%'); out.push(other); }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn get_local_time_offset_secs() -> i32 {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i32
    }
}

#[cfg(target_os = "windows")]
fn get_local_time_offset_secs() -> i32 {
    0
}
