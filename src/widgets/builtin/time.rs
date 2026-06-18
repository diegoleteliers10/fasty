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
}

impl TimeWidget {
    pub fn new(format: String, align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            format,
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            cached: String::new(),
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
        self.cached = format_now(&self.format);
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

fn format_now(fmt: &str) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
