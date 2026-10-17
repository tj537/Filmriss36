//! Look & Feel: dunkles Kamera-UI, große Touch-Ziele.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2};

/// Warmes Orange – erinnert an die Signalfarbe alter Filmdosen.
pub const ACCENT: Color32 = Color32::from_rgb(255, 138, 61);
pub const BG: Color32 = Color32::from_rgb(12, 12, 13);
pub const PANEL: Color32 = Color32::from_rgb(22, 22, 24);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 150, 155);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(10.0, 8.0);
        s.button_padding = Vec2::new(12.0, 7.0);
        // ≥ 32 pt hohe Bedienelemente – mit dem Finger gut treffbar
        s.interact_size = Vec2::new(40.0, 32.0);
        s.slider_width = 170.0;
        s.icon_width = 20.0;
        s.combo_width = 180.0;
        s.scroll.floating = true;

        let v = &mut style.visuals;
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.extreme_bg_color = BG;
        v.selection.bg_fill = ACCENT.linear_multiply(0.6);
        v.selection.stroke = Stroke::new(1.5, ACCENT);
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;
        let r = CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = r;
        }
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.linear_multiply(0.7));
        v.window_corner_radius = CornerRadius::same(12);
    });
}
