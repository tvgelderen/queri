use gpui_kit::test::{TestEventExt, TestQueryExt, TestWindowExt};
use gpui_kit::{App, MouseButton, Window};

pub fn click_label(window: &mut Window, label: &str, cx: &mut App) {
    window.render_frame(cx);
    let element = window.find_by_label(label);
    assert!(element.visible(), "{label} must be visible");
    let position = element.bounds().center();
    window.pointer_move(position, None, cx);
    window.pointer_down(position, MouseButton::Left, cx);
    window.pointer_up(position, MouseButton::Left, cx);
}
