use super::{destructive_menu_item, menu_item};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{button::Button, menu::DropdownMenu};
use gpui_kit::test::{TestAppContextExt, TestEventExt, TestQueryExt, TestWindowExt};
use gpui_kit::{
    AppContext, Context, FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Render, TestAppContext, Window, WindowOptions, div, point, px,
};
use std::time::Duration;

struct MenuFixture {
    deletions: usize,
    focus: FocusHandle,
}

impl Render for MenuFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        div()
            .id("fixture")
            .test_support()
            .track_focus(&self.focus)
            .child(
                Button::new("open-menu")
                    .label("Actions")
                    .dropdown_menu(move |menu, _, _| {
                        let view = view.clone();
                        menu.item(destructive_menu_item("Delete database", false).on_click(
                            move |_, _, cx| {
                                view.update(cx, |this, _| this.deletions += 1);
                            },
                        ))
                        .item(
                            destructive_menu_item("Delete unavailable…", true)
                                .on_click(|_, _, _| panic!("disabled deletion activated")),
                        )
                        .item(menu_item("Keep database", IconName::Check))
                    }),
            )
    }
}

#[gpui_kit::test]
async fn destructive_menu_rows_keep_native_activation_and_disabled_behavior(
    cx: &mut TestAppContext,
) {
    let (window, fixture) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let fixture = cx.new(|cx| MenuFixture {
                deletions: 0,
                focus: cx.focus_handle(),
            });
            fixture.read(cx).focus.clone().focus(window, cx);
            fixture
        })
        .expect("menu fixture")
    });
    cx.update_window(window, |_, window, cx| {
        window.click("open-menu", cx);
        assert!(window.find_by_label("Delete database").visible());
        assert_eq!(
            window.find_by_label("Delete database").value(),
            Some("Delete database"),
            "native menu items derive their accessible name from descendant label values"
        );
        let row = window.within("popup-menu").find(0_usize).bounds();
        let background = window
            .within("popup-menu")
            .within(0_usize)
            .find("destructive-menu-item")
            .bounds();
        assert_eq!(
            background, row,
            "the destructive hover must cover the whole row"
        );
        window.within("popup-menu").click(1_usize, cx);
        assert_eq!(fixture.read(cx).deletions, 0);
        assert!(window.find("popup-menu").visible());
        window.press("down", cx);
        assert_eq!(
            window.within("popup-menu").find(2_usize).selected(),
            Some(true)
        );
        window.press("up", cx);
        assert_eq!(
            window.within("popup-menu").find(0_usize).selected(),
            Some(true)
        );
        window.press("enter", cx);
        assert_eq!(fixture.read(cx).deletions, 1);
    })
    .expect("disabled rows are inert and skipped by keyboard navigation");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(window, |_, window, cx| {
        assert_eq!(window.find("fixture").focused(), Some(true));
        window.click("open-menu", cx);
        let bounds = window.within("popup-menu").find(0_usize).bounds();
        let edge = point(bounds.left() + px(1.), bounds.center().y);
        window.pointer_move(edge, None, cx);
        window.pointer_down(edge, MouseButton::Left, cx);
        window.pointer_up(edge, MouseButton::Left, cx);
        assert_eq!(fixture.read(cx).deletions, 2);
    })
    .expect("the full destructive row remains clickable, including its padding");
    cx.wait_for(window, Duration::from_secs(1), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
}
