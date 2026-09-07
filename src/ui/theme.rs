use gpui_kit::component::Theme;
use gpui_kit::{App, Hsla, px, rgb};
use std::sync::Arc;

/// Apply after changing appearance so component and base-layer tokens stay in sync.
pub fn apply(cx: &mut App) {
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        let color = |light, night| -> Hsla { rgb(if dark { night } else { light }).into() };
        let canvas = color(0xffffff, 0x141414);
        let sidebar = color(0xf5f5f5, 0x101010);
        let surface = color(0xffffff, 0x1c1c1c);
        let subtle = color(0xf8f8f8, 0x1c1c1c);
        let hover = color(0xececec, 0x292929);
        let selected = color(0xe5e5e5, 0x333333);
        let border = color(0xe3e3e3, 0x303030);
        let foreground = color(0x202020, 0xebebeb);
        let secondary = color(0x5f5f5f, 0xa8a8a8);
        let focus = color(0x000000, 0xffffff);
        let primary = color(0x000000, 0xffffff);
        let primary_text = color(0xffffff, 0x000000);

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
        theme.input = color(0x8a8a8a, 0x686868);
        theme.ring = focus;
        theme.caret = foreground;
        theme.selection = focus.opacity(0.22);

        // Components consume both the semantic palette and dedicated button tokens.
        theme.primary = primary;
        theme.primary_foreground = primary_text;
        theme.primary_hover = color(0x262626, 0xe5e5e5);
        theme.primary_active = color(0x404040, 0xd4d4d4);
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
        theme.sidebar_accent = selected;
        theme.sidebar_accent_foreground = primary;
        theme.sidebar_primary = primary;
        theme.sidebar_primary_foreground = primary_text;

        theme.colors.list = surface;
        theme.list_active = selected;
        theme.list_active_border = focus;
        theme.list_hover = hover;
        theme.list_head = subtle;
        theme.list_even = canvas;
        theme.list.active_highlight = true;

        theme.tab = sidebar;
        theme.tab_bar = sidebar;
        theme.tab_bar_segmented = subtle;
        theme.tab_active = if dark { surface } else { selected };
        theme.tab_active_foreground = primary;
        theme.tab_foreground = secondary;

        theme.table = canvas;
        theme.table_head = subtle;
        theme.table_head_foreground = secondary;
        theme.table_foot = canvas;
        theme.table_foot_foreground = secondary;
        theme.table_even = canvas;
        theme.table_hover = hover;
        // GPUI Kit paints cell selection over the content, so the tint must stay translucent.
        theme.table_active = focus.opacity(0.12);
        theme.table_active_border = focus;
        theme.table_row_border = border.opacity(0.55);

        theme.button = surface;
        theme.button_foreground = foreground;
        theme.button_hover = hover;
        theme.button_active = selected;
        theme.danger = color(0xb42318, 0xf48a82);
        theme.danger_foreground = color(0xffffff, 0x181818);
        theme.button_danger = color(0xfef3f2, 0x332322);
        theme.button_danger_hover = color(0xfee4e2, 0x442927);
        theme.button_danger_active = color(0xfecdca, 0x53312e);
        theme.button_danger_foreground = theme.danger;
        theme.group_box = subtle;
        theme.group_box_foreground = foreground;
        theme.title_bar = sidebar;
        theme.title_bar_border = border;
        theme.status_bar = sidebar;
        theme.status_bar_border = border;
        theme.drag_border = focus;
        theme.drop_target = selected;
        theme.skeleton = subtle;
        theme.scrollbar = canvas.opacity(0.);
        theme.scrollbar_thumb = secondary.opacity(0.35);
        theme.scrollbar_thumb_hover = secondary.opacity(0.6);

        let editor = &mut Arc::make_mut(&mut theme.highlight_theme).style;
        editor.editor_background = Some(canvas);
        editor.editor_foreground = Some(foreground);
        editor.editor_gutter_background = Some(canvas);
        editor.editor_active_line = Some(subtle);
        editor.editor_line_number = Some(secondary);
        editor.editor_active_line_number = Some(foreground);
    });
}
