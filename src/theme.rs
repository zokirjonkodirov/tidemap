//! Design tokens and shared surface primitives.
//!
//! Everything in the application chrome should consume these values. The
//! diagram style sheet owns diagram styling; this module owns spacing, radii,
//! type, elevation and the one-accent palette used by the native UI.
use egui::{
    Color32, Context, CornerRadius, FontFamily, Frame, Margin, RichText, Shadow, Stroke, Visuals, vec2,
};
use tidemap_model::Theme;

/// Single source of truth for spacing, shape, type and motion.
pub mod token {
    pub const SPACE: [f32; 6] = [4.0, 8.0, 12.0, 16.0, 24.0, 32.0];

    pub const RADIUS_CONTROL: f32 = 8.0;
    pub const RADIUS_PANEL: f32 = 12.0;
    pub const RADIUS_DIALOG: f32 = 16.0;
    pub const RADIUS_PILL: f32 = 999.0;

    pub const FONT_UI: f32 = 14.0;
    pub const FONT_SMALL: f32 = 13.0;
    pub const FONT_MONO: f32 = 13.0;
    pub const FONT_TITLE: f32 = 20.0;

    pub const BAR_HEIGHT: f32 = 40.0;
    pub const ROW_HEIGHT: f32 = 32.0;
    pub const CONTROL: f32 = 32.0;

    pub const SIDEBAR: f32 = 260.0;
    pub const SIDEBAR_MIN: f32 = 220.0;
    pub const SIDEBAR_MAX: f32 = 360.0;
    pub const SOURCE: f32 = 420.0;
    pub const SOURCE_MIN: f32 = 280.0;
    pub const INSET: f32 = 8.0;

    /// Duration of egui's own animations (scroll, collapsing, tooltips). Our surfaces use
    /// `crate::motion`.
    pub const EASE_SECONDS: f32 = 0.18;
    pub const NARROW: f32 = 1000.0;

}

#[derive(Clone, Copy)]
pub struct Palette {
    pub app: Color32,
    pub panel: Color32,
    pub hover: Color32,
    pub ink: Color32,
    pub secondary: Color32,
    pub muted: Color32,
    pub hairline: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub shadow: Color32,
    /// Backdrop behind dialogs and the palette.
    pub scrim: Color32,
    /// The selected segment of a segmented control.
    pub raised: Color32,
    /// Fill of recessed controls and boxes inside a panel: segmented track, key caps, code boxes.
    pub field: Color32,
    /// Track of a switch that is off.
    pub track_off: Color32,
    /// Knob of a switch that is off.
    pub knob_off: Color32,
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Alpha-composite `top` over `bottom`.
pub fn over(top: Color32, bottom: Color32) -> Color32 {
    let t = top.to_srgba_unmultiplied();
    let b = bottom.to_srgba_unmultiplied();
    let ta = t[3] as f32 / 255.0;
    let ba = b[3] as f32 / 255.0;
    let out_a = ta + ba * (1.0 - ta);
    if out_a <= 0.0 {
        return Color32::TRANSPARENT;
    }
    let mix = |tc: u8, bc: u8| -> u8 {
        let v = (tc as f32 * ta + bc as f32 * ba * (1.0 - ta)) / out_a;
        v.round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_unmultiplied(
        mix(t[0], b[0]),
        mix(t[1], b[1]),
        mix(t[2], b[2]),
        (out_a * 255.0).round() as u8,
    )
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        let app = rgb(0x1B, 0x1D, 0x20);
        Palette {
            app,
            panel: rgb(0x24, 0x27, 0x2B),
            hover: alpha(rgb(255, 255, 255), 15),
            ink: rgb(0xE6, 0xE8, 0xEB),
            secondary: rgb(0xA9, 0xB0, 0xBA),
            muted: rgb(0x90, 0x99, 0xA4),
            hairline: alpha(rgb(255, 255, 255), 20),
            accent: rgb(0x8F, 0xAE, 0xE0),
            accent_soft: rgb(0x2A, 0x34, 0x47),
            on_accent: rgb(0x14, 0x18, 0x1C),
            danger: rgb(0xE1, 0x9A, 0x9A),
            shadow: alpha(rgb(0x14, 0x18, 0x1C), 128),
            scrim: alpha(rgb(0x00, 0x00, 0x00), 110),
            raised: rgb(0x3A, 0x3E, 0x44),
            field: rgb(0x1B, 0x1D, 0x20),
            track_off: rgb(0x4A, 0x4F, 0x56),
            knob_off: rgb(0xD5, 0xD9, 0xDF),
        }
    } else {
        let ink = rgb(0x1F, 0x23, 0x28);
        let app = rgb(0xFA, 0xFA, 0xF8);
        Palette {
            app,
            panel: rgb(0xFF, 0xFF, 0xFF),
            hover: alpha(ink, 13),
            ink,
            secondary: rgb(0x5F, 0x66, 0x72),
            muted: rgb(0x6B, 0x72, 0x80),
            hairline: alpha(ink, 20),
            accent: rgb(0x41, 0x60, 0x9E),
            accent_soft: rgb(0xE9, 0xEE, 0xF7),
            on_accent: rgb(0xFF, 0xFF, 0xFF),
            danger: rgb(0xA2, 0x3B, 0x3B),
            shadow: alpha(rgb(0x14, 0x18, 0x1C), 36),
            scrim: alpha(ink, 31),
            raised: rgb(0xFF, 0xFF, 0xFF),
            field: rgb(0xEC, 0xEE, 0xF1),
            track_off: rgb(0xD3, 0xD6, 0xDB),
            knob_off: rgb(0xFF, 0xFF, 0xFF),
        }
    }
}

pub fn font_medium() -> FontFamily {
    FontFamily::Name("inter-medium".into())
}

pub fn font_semibold() -> FontFamily {
    FontFamily::Name("inter-semibold".into())
}

/// The one shadow, used by floating surfaces only.
fn floating_shadow(p: Palette) -> Shadow {
    Shadow { offset: [0, 8], blur: 24, spread: 0, color: p.shadow }
}

fn widget(visuals: &mut Visuals, p: Palette) {
    let radius = CornerRadius::same(token::RADIUS_CONTROL as u8);
    let w = &mut visuals.widgets;

    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    w.noninteractive.bg_stroke = Stroke::NONE;
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.secondary);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = Color32::TRANSPARENT;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.hovered.corner_radius = radius;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.active.corner_radius = radius;

    w.open.bg_fill = p.accent_soft;
    w.open.weak_bg_fill = p.accent_soft;
    w.open.bg_stroke = Stroke::NONE;
    w.open.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.open.corner_radius = radius;
}

pub fn apply(ctx: &Context, theme: Theme) -> bool {
    let dark = match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => ctx.system_theme() == Some(egui::Theme::Dark),
    };
    let p = palette(dark);
    let mut visuals = if dark { Visuals::dark() } else { Visuals::light() };
    visuals.panel_fill = p.app;
    visuals.window_fill = p.panel;
    visuals.window_stroke = Stroke::NONE;
    visuals.window_corner_radius = CornerRadius::same(token::RADIUS_DIALOG as u8);
    visuals.menu_corner_radius = CornerRadius::same(token::RADIUS_PANEL as u8);
    visuals.window_shadow = floating_shadow(p);
    visuals.popup_shadow = floating_shadow(p);
    visuals.extreme_bg_color = p.app;
    visuals.text_edit_bg_color = Some(p.app);
    visuals.faint_bg_color = p.hover;
    visuals.code_bg_color = p.accent_soft;
    visuals.override_text_color = Some(p.ink);
    visuals.weak_text_color = Some(p.secondary);
    visuals.weak_text_alpha = 1.0;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.accent;
    visuals.error_fg_color = p.danger;
    visuals.selection.bg_fill = p.accent_soft;
    // Keyboard focus ring: the only stroke widgets draw.
    visuals.selection.stroke = Stroke::new(2.0_f32, p.accent);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    widget(&mut visuals, p);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.animation_time = token::EASE_SECONDS;
        style.spacing.item_spacing = vec2(token::SPACE[1], token::SPACE[1]);
        style.spacing.button_padding = vec2(token::SPACE[2], token::SPACE[0] + 2.0);
        style.spacing.interact_size = vec2(token::CONTROL, token::CONTROL);
        style.spacing.window_margin = token::SPACE[3].into();
        style.spacing.menu_margin = token::SPACE[1].into();
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(token::FONT_UI));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(token::FONT_UI));
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(token::FONT_SMALL));
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(token::FONT_TITLE));
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(token::FONT_MONO));
    });
    dark
}

/// Floating surface: panel fill, the one shadow, no stroke.
pub fn floating(p: Palette, radius: f32) -> Frame {
    Frame::new()
        .fill(p.panel)
        .corner_radius(radius)
        .inner_margin(Margin::symmetric(token::SPACE[2] as i8, token::SPACE[1] as i8))
        .shadow(floating_shadow(p))
}

/// Inset surface: panel fill, `RADIUS_PANEL`, no stroke, no shadow.
pub fn inset_panel(p: Palette) -> Frame {
    Frame::new().fill(p.panel).corner_radius(token::RADIUS_PANEL).inner_margin(token::SPACE[3])
}

/// Consistent secondary line used under headings and in footers.
pub fn caption(ui: &mut egui::Ui, p: Palette, text: impl ToString) {
    ui.label(RichText::new(text.to_string()).size(token::FONT_SMALL).color(p.muted));
}
