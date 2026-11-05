//! Floating stack of in-app notifications.
//!
//! Each card shows what happened, where it happened, how long ago, and the
//! actions the user can take without leaving the window. The stack replaces
//! the per-pane status pills: one card per source, newest at the bottom.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, px, Animation, AnimationExt, AnyElement, App, CursorStyle, FontWeight, IntoElement,
    MouseButton, MouseDownEvent, RenderOnce, SharedString, Window,
};

use super::tab_icons::{render_tab_glyph, TabGlyph};
use super::theme::Theme;

/// One card per source stays readable. Older cards drop off the top.
pub const NOTIFICATION_STACK_CAPACITY: usize = 3;

const CARD_WIDTH: f32 = 360.0;
const CARD_PAD_X: f32 = 12.0;

/// A card that asks for nothing clears fast: it only reports that work ended.
const INFORMATIONAL_CARD_LIFETIME: Duration = Duration::from_secs(3);

/// Motion. Enter is a quick fade-in, exit is faster so the stack settles at once.
const ENTER_DURATION: Duration = Duration::from_millis(180);
pub const EXIT_DURATION: Duration = Duration::from_millis(140);

/// A failure stays long enough to read. Working and needs-you cards stay until
/// the source changes state or the user acts on them.
pub const NOTIFICATION_CARD_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationTone {
    Working,
    Done,
    NeedsYou,
    Failed,
}

impl NotificationTone {
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Working => "running",
            Self::Done => "done",
            Self::NeedsYou => "needs you",
            Self::Failed => "failed",
        }
    }

    /// Working and needs-you cards ignore the lifetime: they describe a state
    /// the user still has to look at.
    pub fn persists(self) -> bool {
        matches!(self, Self::Working | Self::NeedsYou)
    }

    /// An informational card clears itself, so it shows a countdown and a bar.
    pub fn expires(self) -> bool {
        !self.persists()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationAction {
    OpenTab,
    Allow,
    Review,
    Dismiss,
}

impl NotificationAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenTab => "Open tab",
            Self::Allow => "Allow",
            Self::Review => "Review",
            Self::Dismiss => "Dismiss",
        }
    }

    pub fn is_primary(self) -> bool {
        matches!(self, Self::OpenTab | Self::Allow)
    }
}

#[derive(Clone, Debug)]
pub struct InternalNotification {
    /// One live card per key. A new event for the same key replaces the card.
    pub key: String,
    /// State signature. A dismissed fingerprint never returns, but a new state
    /// for the same key does.
    pub fingerprint: String,
    pub source: String,
    pub glyph: TabGlyph,
    pub tone: NotificationTone,
    pub detail: String,
    pub created_at: Instant,
    /// Percent, when the source reports progress.
    pub progress: Option<u8>,
    pub actions: Vec<NotificationAction>,
    /// Stable tab identity. Tab positions move when a tab closes, so the card
    /// keeps the id and the action resolves the position at click time.
    pub tab_id: Option<usize>,
    /// Set for cards owned by a terminal pane, so the stack drops them when the
    /// pane closes.
    pub pane_id: Option<usize>,
    /// A repeatable event, such as a finished command, is a new event each
    /// time. A state report is not: the same state keeps the same card.
    pub always_new: bool,
    /// Set when the card starts its exit animation. It leaves the queue once
    /// the animation finishes, not when it starts.
    pub departing: Option<Instant>,
}

impl InternalNotification {
    /// How long this card lives before it clears itself.
    pub fn lifetime(&self) -> Duration {
        match self.tone {
            NotificationTone::Done => INFORMATIONAL_CARD_LIFETIME,
            _ => NOTIFICATION_CARD_LIFETIME,
        }
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        self.tone.expires() && now.saturating_duration_since(self.created_at) > self.lifetime()
    }

    pub fn is_departing(&self) -> bool {
        self.departing.is_some()
    }

    /// Time left before the card clears itself. `None` for a card that waits
    /// for the user.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.tone.expires().then(|| {
            self.lifetime()
                .saturating_sub(now.saturating_duration_since(self.created_at))
        })
    }
}

/// The live cards plus the memory of what the user dismissed.
///
/// A state card (`always_new == false`) stays dismissed until its state
/// changes. An event card (`always_new == true`) is a new event each time, so
/// dismissing it never suppresses the next one.
#[derive(Debug, Default)]
pub struct NotificationQueue {
    cards: Vec<InternalNotification>,
    dismissed: std::collections::HashMap<String, String>,
}

impl NotificationQueue {
    pub fn cards(&self) -> &[InternalNotification] {
        &self.cards
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn position(&self, key: &str) -> Option<usize> {
        self.cards.iter().position(|card| card.key == key)
    }

    /// Returns true when the card reached the stack.
    pub fn push(&mut self, mut card: InternalNotification) -> bool {
        if !card.always_new && self.dismissed.get(&card.key) == Some(&card.fingerprint) {
            return false;
        }
        // Same state, new tick: keep the original timestamp so the card shows
        // how long the state has lasted, not how long since the last refresh.
        // A repeatable event is a new event, so it keeps its own clock.
        if !card.always_new && card.tone != NotificationTone::Done {
            if let Some(existing) = self.cards.iter().find(|existing| {
                existing.key == card.key
                    && existing.fingerprint == card.fingerprint
                    && existing.departing.is_none()
            }) {
                card.created_at = existing.created_at;
            }
        }
        // The permission prompt is single-slot: a new request replaces the
        // card of the request it replaced in the app state.
        if card.key.starts_with("ai-perm:") {
            self.cards
                .retain(|existing| !existing.key.starts_with("ai-perm:"));
        }
        self.cards.retain(|existing| existing.key != card.key);
        self.cards.push(card);
        while self.cards.len() > NOTIFICATION_STACK_CAPACITY {
            // Attention first: a request for permission outlives a finished
            // command when the stack is full.
            let drop_index = self
                .cards
                .iter()
                .position(|card| {
                    matches!(card.tone, NotificationTone::Done | NotificationTone::Failed)
                })
                .unwrap_or(0);
            self.cards.remove(drop_index);
        }
        true
    }

    pub fn dismiss(&mut self, index: usize) -> Option<InternalNotification> {
        if index >= self.cards.len() {
            return None;
        }
        let card = self.cards.get_mut(index)?;
        if card.departing.is_some() {
            return None;
        }
        // Record the dismissal as soon as the exit starts, so a rapid re-push
        // cannot undo it.
        if !card.always_new
            && matches!(card.tone, NotificationTone::Done | NotificationTone::Failed)
        {
            if self.dismissed.len() >= 64 {
                self.dismissed.clear();
            }
            self.dismissed
                .insert(card.key.clone(), card.fingerprint.clone());
        }
        card.departing = Some(Instant::now());
        Some(card.clone())
    }

    pub fn dismiss_key(&mut self, key: &str) {
        if let Some(index) = self.position(key) {
            self.dismiss(index);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.cards.retain(|card| card.key != key);
    }

    pub fn remove_with_prefix(&mut self, prefix: &str) {
        self.cards.retain(|card| !card.key.starts_with(prefix));
    }

    pub fn retain(&mut self, keep: impl Fn(&InternalNotification) -> bool) {
        self.cards.retain(|card| keep(card));
    }

    /// Start the exit animation for cards whose lifetime ended. Returns true
    /// when any card is still visible.
    pub fn expire(&mut self, now: Instant) -> bool {
        for card in &mut self.cards {
            if card.departing.is_none() && card.is_expired(now) {
                card.departing = Some(now);
            }
        }
        !self.cards.is_empty()
    }

    /// Remove the cards whose exit animation finished.
    pub fn prune_departed(&mut self, now: Instant) {
        self.cards.retain(|card| {
            card.departing
                .is_none_or(|started| now.saturating_duration_since(started) < EXIT_DURATION)
        });
    }

    pub fn has_departing(&self) -> bool {
        self.cards.iter().any(InternalNotification::is_departing)
    }
}

/// `in 3s`: how long an informational card stays.
pub fn format_countdown(remaining: Duration) -> String {
    format!("in {}", format_elapsed(remaining))
}

/// `51s`, `4m 03s`, `2h 07m`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Keep a card readable on one line: flatten the line breaks and cut the tail.
pub fn summarize_detail(text: &str, max_chars: usize) -> String {
    let flat = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let kept: String = flat.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{}...", kept.trim_end())
}

pub type NotificationActionCallback =
    Rc<dyn Fn(&(String, NotificationAction), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct NotificationStack {
    cards: Vec<InternalNotification>,
    theme: Theme,
    now: Instant,
    max_width: f32,
    on_action: Option<NotificationActionCallback>,
}

impl NotificationStack {
    pub fn new(
        cards: Vec<InternalNotification>,
        theme: Theme,
        now: Instant,
        max_width: f32,
    ) -> Self {
        Self {
            cards,
            theme,
            now,
            max_width,
            on_action: None,
        }
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(&(String, NotificationAction), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for NotificationStack {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let callback = self.on_action;
        let reduce_motion = _cx.reduce_motion();
        let mut stack = div()
            .id("notification-stack")
            .flex()
            .flex_col()
            .items_end()
            .gap(px(10.))
            // Clicks belong to the card, but the wheel keeps scrolling the
            // panel under the stack.
            .block_mouse_except_scroll();
        let card_width = self.max_width.min(CARD_WIDTH).max(180.0);
        for card in self.cards.iter() {
            stack = stack.child(render_card(
                card,
                card_width,
                self.theme,
                self.now,
                reduce_motion,
                callback.as_ref(),
            ));
        }
        stack
    }
}

fn render_card(
    card: &InternalNotification,
    card_width: f32,
    theme: Theme,
    now: Instant,
    reduce_motion: bool,
    on_action: Option<&NotificationActionCallback>,
) -> impl IntoElement {
    let on_action = if card.is_departing() { None } else { on_action };
    let mut detail = summarize_detail(&card.detail, 120);
    if let (NotificationTone::Working, Some(percent)) = (card.tone, card.progress) {
        if detail.is_empty() {
            detail = format!("{percent}%");
        } else {
            detail = format!("{detail} · {percent}%");
        }
    }
    let stamp = match card.remaining(now) {
        Some(remaining) => format_countdown(remaining),
        None => format_elapsed(now.saturating_duration_since(card.created_at)),
    };
    let mut body = div()
        .id(SharedString::from(format!(
            "notification-card-{}",
            card.key
        )))
        .relative()
        .w(px(card_width))
        .flex()
        .flex_col()
        .gap(px(7.))
        .px(px(CARD_PAD_X))
        .pt(px(11.))
        .pb(px(12.))
        .rounded(px(10.))
        .bg(theme.surface_raised)
        .border_1()
        .border_color(theme.border)
        .shadow_xl()
        .occlude()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(7.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.))
                        .h(px(16.))
                        .flex_shrink_0()
                        .child(render_tab_glyph(card.glyph, theme, true, 14.0)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .truncate()
                        .child(SharedString::from(card.source.clone())),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .flex_shrink_0()
                        .child(tone_icon(card.tone, theme, &card.key))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(tone_color(card.tone, theme))
                                .whitespace_nowrap()
                                .child(card.tone.status_label()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(theme.muted)
                        .whitespace_nowrap()
                        .child(SharedString::from(stamp)),
                ),
        )
        .when(!detail.is_empty(), |card_view| {
            card_view.child(
                div()
                    .pl(px(23.))
                    .text_size(px(12.))
                    .text_color(theme.muted_strong)
                    .truncate()
                    .child(SharedString::from(detail)),
            )
        })
        .when(!card.actions.is_empty(), |card_view| {
            let mut row = div()
                .pl(px(23.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.));
            for action in card.actions.iter().copied() {
                row = row.child(action_button(&card.key, action, theme, on_action.cloned()));
            }
            card_view.child(row)
        });

    // The bar belongs to an informational card: no actions, and it clears
    // itself. A card with actions waits for the user instead.
    if card.actions.is_empty() {
        if let Some(remaining) = card.remaining(now) {
            let fraction =
                (remaining.as_secs_f32() / card.lifetime().as_secs_f32()).clamp(0.0, 1.0);
            body = body.child(lifetime_bar(
                fraction,
                card.lifetime(),
                tone_color(card.tone, theme),
                card_width,
                &card.key,
                &card.fingerprint,
                reduce_motion,
            ));
        }
    }

    let departing = card.departing;
    let animation_id = if departing.is_some() {
        SharedString::from(format!("notification-out-{}", card.key))
    } else {
        SharedString::from(format!("notification-in-{}", card.key))
    };
    let animation = if departing.is_some() {
        Animation::new(EXIT_DURATION).with_easing(gpui::ease_out_quint())
    } else {
        Animation::new(ENTER_DURATION).with_easing(gpui::ease_out_quint())
    };
    body.with_animation(animation_id, animation, move |element, progress| {
        let opacity = if departing.is_some() {
            1.0 - progress
        } else {
            progress
        };
        element.opacity(opacity.clamp(0.0, 1.0))
    })
}

/// The bar counts down the time an informational card has left. It sits on the
/// bottom edge and shrinks from the left, so the card looks like it is fading.
/// Countdown bar. It runs linearly from the current fraction to zero over the
/// time the card has left, so it moves continuously instead of stepping once
/// per second. Linear easing is correct here: a countdown is constant motion.
fn lifetime_bar(
    fraction: f32,
    lifetime: Duration,
    color: gpui::Hsla,
    card_width: f32,
    key: &str,
    fingerprint: &str,
    reduce_motion: bool,
) -> AnyElement {
    let track = (card_width - 2.0 * CARD_PAD_X).max(0.0);
    let start_width = (track * fraction.clamp(0.0, 1.0)).max(0.0);
    // The animation duration must not change between renders: gpui keeps the
    // start instant per element id, so a shrinking duration would make the
    // progress jump. The lifetime is constant for the tone, and a fresh event
    // gets a fresh id from the fingerprint so its bar starts over.
    let duration = lifetime.max(Duration::from_millis(80));
    let serial = seahash::hash(fingerprint.as_bytes());
    let bar = div()
        .absolute()
        .bottom(px(2.))
        .left(px(CARD_PAD_X))
        .h(px(3.))
        .w(px(start_width))
        .rounded(px(2.))
        .bg(color);
    if reduce_motion {
        // Without motion the bar still reports the time left, updated by the
        // one-second tick.
        return bar.into_any_element();
    }
    bar.with_animation(
        SharedString::from(format!("notification-bar-{key}-{serial}")),
        // A bar that empties in seconds needs 60 fps; a long one moves a
        // fraction of a pixel per frame, so keep the repaint cheap.
        Animation::new(duration)
            .with_easing(gpui::linear)
            .with_max_fps(if duration < Duration::from_secs(6) {
                60.0
            } else {
                12.0
            }),
        move |element, progress| element.w(px((start_width * (1.0 - progress)).max(0.0))),
    )
    .into_any_element()
}

fn tone_icon(tone: NotificationTone, theme: Theme, key: &str) -> impl IntoElement {
    let color = tone_color(tone, theme);
    match tone {
        NotificationTone::Working => div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(super::icons::render_spinner(
                color,
                12.0,
                SharedString::from(format!("notification-spinner-{key}")),
            ))
            .into_any_element(),
        NotificationTone::Done => div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(super::icons::render_icon(
                icons::common::IconType::Check,
                color,
                12.0,
            ))
            .into_any_element(),
        NotificationTone::NeedsYou => div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(super::icons::render_icon(
                icons::common::IconType::HandHelping,
                color,
                12.0,
            ))
            .into_any_element(),
        NotificationTone::Failed => div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(super::icons::render_icon(
                icons::common::IconType::X,
                color,
                12.0,
            ))
            .into_any_element(),
    }
}

fn tone_color(tone: NotificationTone, theme: Theme) -> gpui::Hsla {
    match tone {
        NotificationTone::Working => theme.accent,
        NotificationTone::Done => theme.green,
        NotificationTone::NeedsYou => theme.yellow,
        NotificationTone::Failed => theme.red,
    }
}

fn action_button(
    key: &str,
    action: NotificationAction,
    theme: Theme,
    on_action: Option<NotificationActionCallback>,
) -> impl IntoElement {
    let primary = action.is_primary();
    let key = key.to_string();
    let button = div()
        .id(SharedString::from(format!(
            "notification-action-{key}-{}",
            action.label()
        )))
        .px(if primary { px(10.) } else { px(7.) })
        .py(px(4.))
        .rounded(px(6.))
        .text_size(px(11.5))
        .font_weight(if primary {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .text_color(if primary {
            theme.foreground
        } else {
            theme.muted_strong
        })
        .cursor(CursorStyle::PointingHand)
        .hover(move |style| {
            if primary {
                style.bg(theme.selected)
            } else {
                style.bg(theme.hover).text_color(theme.foreground)
            }
        })
        .when(primary, |style| style.bg(theme.selected));
    button.child(action.label()).on_mouse_down(
        MouseButton::Left,
        move |_event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if let Some(callback) = on_action.as_ref() {
                callback(&(key.clone(), action), window, cx);
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(key: &str, fingerprint: &str, tone: NotificationTone) -> InternalNotification {
        InternalNotification {
            key: key.to_string(),
            fingerprint: fingerprint.to_string(),
            source: "cargo test".to_string(),
            glyph: TabGlyph::Shell,
            tone,
            detail: String::new(),
            created_at: Instant::now(),
            progress: None,
            actions: Vec::new(),
            tab_id: None,
            pane_id: None,
            always_new: false,
            departing: None,
        }
    }

    #[test]
    fn elapsed_reads_in_seconds_minutes_and_hours() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(format_elapsed(Duration::from_secs(51)), "51s");
        assert_eq!(format_elapsed(Duration::from_secs(243)), "4m 03s");
        assert_eq!(format_elapsed(Duration::from_secs(7_620)), "2h 07m");
    }

    #[test]
    fn a_countdown_reads_as_time_remaining() {
        assert_eq!(format_countdown(Duration::from_secs(3)), "in 3s");
        assert_eq!(format_countdown(Duration::from_secs(243)), "in 4m 03s");
    }

    #[test]
    fn only_informational_cards_count_down() {
        let now = Instant::now();
        let mut done = card("done", "1", NotificationTone::Done);
        done.created_at = now - Duration::from_secs(1);
        assert_eq!(
            done.remaining(now),
            Some(INFORMATIONAL_CARD_LIFETIME - Duration::from_secs(1))
        );

        let mut needs_you = card("needs-you", "1", NotificationTone::NeedsYou);
        needs_you.created_at = now - Duration::from_secs(9);
        assert_eq!(needs_you.remaining(now), None);

        let mut working = card("working", "1", NotificationTone::Working);
        working.created_at = now - Duration::from_secs(9);
        assert_eq!(working.remaining(now), None);
    }

    #[test]
    fn detail_flattens_lines_and_cuts_the_tail() {
        assert_eq!(
            summarize_detail("Fixed osc_title\n\n2 files changed", 96),
            "Fixed osc_title 2 files changed"
        );
        assert_eq!(summarize_detail("abcdef", 4), "a...");
        assert_eq!(summarize_detail("  ", 10), "");
    }

    #[test]
    fn finished_cards_expire_and_attention_cards_do_not() {
        let created_at = Instant::now();
        let card = |tone| InternalNotification {
            key: "k".to_string(),
            fingerprint: "f".to_string(),
            source: "cargo test".to_string(),
            glyph: TabGlyph::Shell,
            tone,
            detail: String::new(),
            created_at,
            progress: None,
            actions: Vec::new(),
            tab_id: None,
            pane_id: None,
            always_new: false,
            departing: None,
        };
        let later = created_at + NOTIFICATION_CARD_LIFETIME + Duration::from_secs(1);
        assert!(card(NotificationTone::Done).is_expired(later));
        assert!(card(NotificationTone::Failed).is_expired(later));
        assert!(!card(NotificationTone::NeedsYou).is_expired(later));
        assert!(!card(NotificationTone::Working).is_expired(later));
    }

    #[test]
    fn a_done_card_clears_after_three_seconds() {
        let started = Instant::now();
        let mut done = card("done", "1", NotificationTone::Done);
        done.created_at = started;
        assert!(!done.is_expired(started + Duration::from_secs(2)));
        assert!(done.is_expired(started + Duration::from_secs(4)));

        let mut failed = card("failed", "1", NotificationTone::Failed);
        failed.created_at = started;
        assert!(!failed.is_expired(started + Duration::from_secs(4)));
    }

    #[test]
    fn one_card_per_key_and_the_oldest_leaves_at_capacity() {
        let mut queue = NotificationQueue::default();
        queue.push(card("a", "1", NotificationTone::Done));
        queue.push(card("b", "1", NotificationTone::Done));
        queue.push(card("a", "2", NotificationTone::Failed));
        assert_eq!(queue.cards().len(), 2);
        assert_eq!(queue.cards()[1].key, "a");
        assert_eq!(queue.cards()[1].tone, NotificationTone::Failed);

        for key in ["c", "d", "e"] {
            queue.push(card(key, "1", NotificationTone::Done));
        }
        let keys: Vec<&str> = queue.cards().iter().map(|card| card.key.as_str()).collect();
        assert_eq!(keys, ["c", "d", "e"]);
    }

    #[test]
    fn a_dismissed_working_card_comes_back_on_the_next_report() {
        let mut queue = NotificationQueue::default();
        queue.push(card("pane:1", "working", NotificationTone::Working));
        queue.dismiss(0);
        assert!(queue.cards()[0].is_departing());
        queue.prune_departed(Instant::now() + EXIT_DURATION + Duration::from_millis(1));
        assert!(queue.is_empty());

        // A permission or working card must never be suppressed: the next
        // report is a new event for the user.
        queue.push(card("pane:1", "working", NotificationTone::Working));
        assert_eq!(queue.cards().len(), 1);
    }

    #[test]
    fn a_dismissed_result_stays_dismissed_until_it_changes() {
        let mut queue = NotificationQueue::default();
        queue.push(card("pane:1", "done", NotificationTone::Done));
        queue.dismiss(0);
        queue.prune_departed(Instant::now() + EXIT_DURATION + Duration::from_millis(1));
        assert!(queue.is_empty());

        queue.push(card("pane:1", "done", NotificationTone::Done));
        assert!(queue.is_empty(), "the same result must not come back");

        queue.push(card("pane:1", "error", NotificationTone::Failed));
        assert_eq!(queue.cards().len(), 1);
    }

    #[test]
    fn a_dismissed_repeatable_event_comes_back_on_the_next_event() {
        let mut queue = NotificationQueue::default();
        let mut finished = card("command:1", "exit 0", NotificationTone::Done);
        finished.always_new = true;
        queue.push(finished.clone());
        queue.dismiss(0);
        queue.prune_departed(Instant::now() + EXIT_DURATION + Duration::from_millis(1));
        assert!(queue.is_empty());

        queue.push(finished);
        assert_eq!(queue.cards().len(), 1);
    }

    #[test]
    fn the_same_state_keeps_its_original_timestamp() {
        let mut queue = NotificationQueue::default();
        let first = card("pane:1", "working", NotificationTone::Working);
        let created_at = first.created_at;
        queue.push(first);
        std::thread::sleep(Duration::from_millis(5));
        queue.push(card("pane:1", "working", NotificationTone::Working));
        assert_eq!(queue.cards()[0].created_at, created_at);
    }

    #[test]
    fn expiry_drops_finished_cards_and_keeps_attention_cards() {
        let mut queue = NotificationQueue::default();
        queue.push(card("done", "1", NotificationTone::Done));
        queue.push(card("needs-you", "1", NotificationTone::NeedsYou));
        let later = Instant::now() + NOTIFICATION_CARD_LIFETIME + Duration::from_secs(1);
        assert!(queue.expire(later));
        assert!(queue.cards()[0].is_departing());
        queue.prune_departed(later + EXIT_DURATION + Duration::from_millis(1));
        assert_eq!(queue.cards().len(), 1);
        assert_eq!(queue.cards()[0].key, "needs-you");
    }

    #[test]
    fn a_departing_card_survives_until_its_animation_ends() {
        let mut queue = NotificationQueue::default();
        queue.push(card("pane:1", "done", NotificationTone::Done));
        queue.dismiss(0);
        assert!(queue.has_departing());
        queue.prune_departed(Instant::now());
        assert_eq!(queue.cards().len(), 1, "young exit is kept");
        queue.prune_departed(Instant::now() + EXIT_DURATION + Duration::from_millis(1));
        assert!(queue.is_empty());
        assert!(!queue.has_departing());
    }

    #[test]
    fn a_permission_card_survives_a_full_stack() {
        let mut queue = NotificationQueue::default();
        queue.push(card("ai-perm:1", "needs-you", NotificationTone::NeedsYou));
        for index in 0..4 {
            queue.push(card(
                &format!("command:{index}"),
                "exit 0",
                NotificationTone::Done,
            ));
        }
        assert_eq!(queue.cards().len(), NOTIFICATION_STACK_CAPACITY);
        assert!(queue.position("ai-perm:1").is_some());
    }

    #[test]
    fn a_new_permission_card_replaces_the_previous_one() {
        let mut queue = NotificationQueue::default();
        queue.push(card("ai-perm:1", "needs-you|a", NotificationTone::NeedsYou));
        queue.push(card("ai-perm:2", "needs-you|b", NotificationTone::NeedsYou));
        assert_eq!(queue.cards().len(), 1);
        assert_eq!(queue.cards()[0].key, "ai-perm:2");
    }

    #[test]
    fn a_repeatable_event_keeps_its_own_clock() {
        let mut queue = NotificationQueue::default();
        let mut first = card("command:1", "exit 1", NotificationTone::Failed);
        first.always_new = true;
        queue.push(first);
        let first_at = queue.cards()[0].created_at;
        std::thread::sleep(Duration::from_millis(5));
        let mut second = card("command:1", "exit 1", NotificationTone::Failed);
        second.always_new = true;
        queue.push(second);
        assert!(queue.cards()[0].created_at > first_at);
    }

    #[test]
    fn every_tone_has_a_label_and_a_primary_action() {
        assert_eq!(NotificationTone::Done.status_label(), "done");
        assert_eq!(NotificationTone::NeedsYou.status_label(), "needs you");
        assert_eq!(NotificationTone::Failed.status_label(), "failed");
        assert!(NotificationAction::Allow.is_primary());
        assert!(NotificationAction::OpenTab.is_primary());
        assert!(!NotificationAction::Dismiss.is_primary());
        assert!(!NotificationAction::Review.is_primary());
    }
}
