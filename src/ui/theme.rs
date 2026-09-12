use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::{App, Hsla, px, rgb};
use std::sync::Arc;

/// The logo's tail cell. Reserved for brand graphics and focus, not text.
pub const BRAND_BLUE: u32 = 0x2f7bff;

/// Brand blue for text and small icons, at readable contrast in both appearances.
pub fn accent_text(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0x6ea2ff
    } else {
        0x1d64e0
    })
    .into()
}

/// A quiet brand tint for selected rows, active filters and badges.
pub fn accent_soft(cx: &App) -> Hsla {
    Hsla::from(rgb(BRAND_BLUE)).opacity(if cx.theme().is_dark() { 0.16 } else { 0.09 })
}

/// Apply after changing appearance so component and base-layer tokens stay in sync.
pub fn apply(cx: &mut App) {
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        let color = |light, night| -> Hsla { rgb(if dark { night } else { light }).into() };
        // Warm neutrals follow the logo ink (#242422); one blue comes from its tail.
        let canvas = color(0xffffff, 0x131313);
        let sidebar = color(0xf6f6f4, 0x0e0e0e);
        let surface = color(0xffffff, 0x1a1a1a);
        let subtle = color(0xf7f7f5, 0x191919);
        let hover = color(0xefefec, 0x222222);
        let selected = color(0xe7e7e4, 0x2a2a2a);
        let border = color(0xe7e7e3, 0x262626);
        let foreground = color(0x1c1c1a, 0xededed);
        let secondary = color(0x6b6b68, 0x9a9a9a);
        let brand: Hsla = rgb(BRAND_BLUE).into();
        // White text on #256ae8 keeps 4.9:1 contrast; the pure logo blue is only 3.9:1.
        let primary: Hsla = rgb(0x256ae8).into();
        let primary_text: Hsla = rgb(0xffffff).into();
        let brand_soft = brand.opacity(if dark { 0.16 } else { 0.09 });

        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.dialog_backdrop_blur = px(8.);
        theme.mono_font_size = px(13.);

        theme.background = canvas;
        theme.foreground = foreground;
        theme.border = border;
        theme.muted = subtle;
        theme.muted_foreground = secondary;
        theme.accent = hover;
        theme.accent_foreground = foreground;
        theme.popover = surface;
        theme.popover_foreground = foreground;
        // GPUI Kit also derives dark control backgrounds from the input border color.
        theme.input = color(0xd6d6d2, 0x3a3a3a);
        theme.ring = brand;
        theme.caret = color(0x256ae8, 0x6ea2ff);
        theme.selection = brand.opacity(if dark { 0.32 } else { 0.22 });
        theme.link = color(0x1d64e0, 0x6ea2ff);
        theme.link_hover = color(0x1a5cd0, 0x8fb4ff);
        theme.link_active = theme.link_hover;

        // Components consume both the semantic palette and dedicated button tokens.
        theme.primary = primary;
        theme.primary_foreground = primary_text;
        theme.primary_hover = color(0x1f5fd6, 0x2f74f2);
        theme.primary_active = color(0x1a55c2, 0x1f5fd6);
        theme.button_primary = primary;
        theme.button_primary_foreground = primary_text;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
        theme.secondary = subtle;
        theme.secondary_foreground = foreground;
        theme.secondary_hover = hover;
        theme.secondary_active = selected;
        theme.button_secondary = subtle;
        theme.button_secondary_foreground = foreground;
        theme.button_secondary_hover = hover;
        theme.button_secondary_active = selected;

        theme.sidebar = sidebar;
        theme.sidebar_foreground = foreground;
        theme.sidebar_border = border;
        theme.sidebar_accent = brand_soft;
        theme.sidebar_accent_foreground = foreground;
        theme.sidebar_primary = primary;
        theme.sidebar_primary_foreground = primary_text;

        theme.colors.list = surface;
        theme.list_active = brand_soft;
        theme.list_active_border = brand.opacity(0.0);
        theme.list_hover = hover;
        theme.list_head = subtle;
        theme.list_even = canvas;
        theme.list.active_highlight = true;

        theme.tab = sidebar;
        theme.tab_bar = sidebar;
        theme.tab_bar_segmented = color(0xeeeeeb, 0x1d1d1d);
        theme.tab_active = canvas;
        theme.tab_active_foreground = foreground;
        theme.tab_foreground = secondary;

        theme.table = canvas;
        theme.table_head = canvas;
        theme.table_head_foreground = secondary;
        theme.table_foot = canvas;
        theme.table_foot_foreground = secondary;
        theme.table_even = canvas;
        theme.table_hover = color(0xf7f7f5, 0x1a1a1a);
        // GPUI Kit paints cell selection over the content, so the tint must stay translucent.
        theme.table_active = brand.opacity(if dark { 0.18 } else { 0.1 });
        theme.table_active_border = brand;
        theme.table_row_border = border.opacity(0.7);

        theme.button = surface;
        theme.button_foreground = foreground;
        theme.button_hover = hover;
        theme.button_active = selected;
        theme.danger = color(0xc0362c, 0xf48a82);
        theme.danger_foreground = color(0xffffff, 0x181818);
        theme.button_danger = color(0xfdf1f0, 0x2e1f1e);
        theme.button_danger_hover = color(0xfbe3e1, 0x3d2523);
        theme.button_danger_active = color(0xf8cfcb, 0x4b2b29);
        theme.button_danger_foreground = theme.danger;
        theme.success = color(0x15803d, 0x7fd8a4);
        theme.success_foreground = color(0xffffff, 0x0e0e0e);
        theme.warning = color(0xb45309, 0xfbbf24);
        theme.warning_foreground = color(0xffffff, 0x0e0e0e);
        theme.info = color(0x1d64e0, 0x6ea2ff);
        theme.group_box = subtle;
        theme.group_box_foreground = foreground;
        theme.title_bar = sidebar;
        theme.title_bar_border = border;
        theme.status_bar = sidebar;
        theme.status_bar_border = border;
        theme.drag_border = brand;
        theme.drop_target = brand_soft;
        theme.skeleton = subtle;
        theme.switch = color(0xd6d6d2, 0x3a3a3a);
        theme.progress_bar = brand;
        theme.slider_bar = brand;
        theme.scrollbar = canvas.opacity(0.);
        theme.scrollbar_thumb = secondary.opacity(0.3);
        theme.scrollbar_thumb_hover = secondary.opacity(0.55);

        let editor = &mut Arc::make_mut(&mut theme.highlight_theme).style;
        editor.editor_background = Some(canvas);
        editor.editor_foreground = Some(foreground);
        editor.editor_gutter_background = Some(canvas);
        editor.editor_active_line = Some(color(0xf8f8f6, 0x181818));
        editor.editor_line_number = Some(secondary.opacity(0.7));
        editor.editor_active_line_number = Some(foreground);
    });
}
