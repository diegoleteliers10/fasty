use std::ops::Range;

use gpui::{
    App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, GlobalElementId, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, Pixels,
    Point, Position, Style, UTF16Selection, Window,
};

use super::root_view::RootView;

/// Returns true when a key stroke should be left to the platform input
/// method instead of being processed by the terminal.
///
/// On macOS, printable keys are sent to the input context
/// (`NSTextInputContext`) so the layout produces the real character:
/// plain keys type as-is, dead keys compose (e.g. `´` + `a` = `á`), and
/// Option-modified keys yield the layout character (e.g. option+n = `~`
/// on the Spanish Latam layout). Routing these keys through the input
/// method also prevents the terminal from writing a key twice: once
/// directly and once from the IME commit.
///
/// When `option_as_meta` is set, Option keys are treated as Alt instead
/// (they emit `ESC` + key), mirroring zed's `option_as_meta` and ghostty's
/// `macos-option-as-alt`.
pub fn defers_to_ime(event: &KeyDownEvent, option_as_meta: bool) -> bool {
    let keystroke = &event.keystroke;
    let Some(key_char) = keystroke.key_char.as_deref() else {
        return false;
    };
    let is_alt_gr = keystroke.modifiers.control && keystroke.modifiers.alt;
    if (keystroke.modifiers.control && !is_alt_gr)
        || keystroke.modifiers.function
        || keystroke.modifiers.platform
    {
        return false;
    }
    if keystroke.modifiers.alt && !is_alt_gr && option_as_meta {
        return false;
    }
    !key_char.is_empty() && key_char.chars().all(|c| !c.is_control())
}

/// Builds an invisible element that registers the terminal view as the
/// platform input handler for the current frame.
pub fn registration(view: Entity<RootView>, focus_handle: FocusHandle) -> ImeRegistration {
    ImeRegistration { view, focus_handle }
}

/// Invisible, full-size element whose only job is to call
/// [`Window::handle_input`] during paint so the platform IME can reach
/// the terminal view.
pub struct ImeRegistration {
    view: Entity<RootView>,
    focus_handle: FocusHandle,
}

impl IntoElement for ImeRegistration {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ImeRegistration {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            position: Position::Absolute,
            size: gpui::Size::full(),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn utf16_slice(text: &str, range: Range<usize>) -> String {
    let units: Vec<u16> = text
        .encode_utf16()
        .skip(range.start)
        .take(range.end.saturating_sub(range.start))
        .collect();
    String::from_utf16_lossy(&units)
}

impl EntityInputHandler for RootView {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .ime_marked_text
            .as_deref()
            .map(utf16_len)
            .unwrap_or(0);
        Some(UTF16Selection {
            range: 0..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_marked_text
            .as_ref()
            .map(|text| 0..utf16_len(text))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.ime_marked_text.as_ref()?;
        *adjusted_range = Some(0..utf16_len(text));
        Some(utf16_slice(text, range))
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_commit_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_set_marked_text(new_text, cx);
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_clear_marked_text(cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.ime_marked_text.as_deref().map(utf16_len).unwrap_or(0))
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.ime_marked_text.as_deref().map(utf16_len).unwrap_or(0))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        true
    }
}