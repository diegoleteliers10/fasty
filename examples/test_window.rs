use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, QuitMode, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, div, prelude::*, px, size,
    Context, Render, Window, IntoElement,
};
use gpui_platform::application;

actions!(test_window, [Quit]);

struct TestView {
    count: usize,
}

impl Render for TestView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(gpui::rgb(0x1e1e2e))
            .text_color(gpui::rgb(0xcdd6f4))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p(px(20.))
                    .bg(gpui::rgb(0x313244))
                    .rounded(px(8.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .child(format!("GPUI Test Window - Count: {}", self.count)),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(8.))
                            .bg(gpui::rgb(0x89b4fa))
                            .text_color(gpui::rgb(0x11111b))
                            .rounded(px(4.))
                            .cursor(gpui::CursorStyle::PointingHand)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                this.count += 1;
                                cx.notify();
                            }))
                            .child("Click Me!"),
                    ),
            )
    }
}

fn main() {
    application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            cx.set_menus([Menu::new("Test").items([MenuItem::action(
                "Quit",
                Quit,
            )])]);

            let bounds = Bounds::centered(None, size(px(600.), px(400.)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(400.), px(300.))),
                    window_background: WindowBackgroundAppearance::Opaque,
                    titlebar: Some(TitlebarOptions {
                        title: Some("GPUI Test".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    cx.new(|_cx| TestView { count: 0 })
                },
            )
            .unwrap();

            cx.activate(true);
        });
}
