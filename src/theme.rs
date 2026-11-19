//! Design tokens and shared surface primitives.
//!
//! Everything in the application chrome should consume these values. The
//! diagram style sheet owns diagram styling; this module owns spacing, radii,
//! type, elevation and the one-accent palette used by the native UI.
use egui::{
    Color32, Context, CornerRadius, FontFamily, Frame, InnerResponse, Margin, Painter, Rect, RichText,
    Shadow, Shape, Stroke, StrokeKind, Ui, Visuals, vec2,
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
    /// True for the dark palette.
    pub dark: bool,
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
            dark: true,
            app,
            panel: rgb(0x24, 0x27, 0x2B),
            hover: alpha(rgb(255, 255, 255), 15),
            ink: rgb(0xE6, 0xE8, 0xEB),
            secondary: rgb(0xA9, 0xB0, 0xBA),
            muted: rgb(0x98, 0xA1, 0xAC),
            hairline: alpha(rgb(255, 255, 255), 20),
            accent: rgb(0x8F, 0xAE, 0xE0),
            accent_soft: rgb(0x2A, 0x34, 0x47),
            on_accent: rgb(0x14, 0x18, 0x1C),
            danger: rgb(0xE1, 0x9A, 0x9A),
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
            dark: false,
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

/// How high a surface sits above the backdrop. Each level has its own fill, shadows and edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Sidebar, source and outline cards.
    Card,
    /// Pill, toast, minimap and other small floating controls.
    Raised,
    /// Popovers, menus and tooltips.
    Overlay,
    /// The command palette and dialogs.
    Modal,
}

/// Everything needed to paint a surface at one [`Level`].
#[derive(Clone, Copy)]
pub struct Elevation {
    /// Surface fill.
    pub fill: Color32,
    /// Tight shadow that anchors the surface to what is under it.
    pub contact: Shadow,
    /// Wide shadow that says how high the surface floats.
    pub key: Shadow,
    /// 1 px edge drawn inside the surface.
    pub edge: Stroke,
    /// Light line along the top edge (dark mode only; transparent in light mode).
    pub highlight: Color32,
}

/// `pct` percent opacity as an alpha byte.
fn percent(pct: u8) -> u8 {
    (255.0 * f32::from(pct) / 100.0).round() as u8
}

/// A shadow dropped straight down by `y`, tinted `color` at `pct` percent.
fn drop_shadow(y: i8, blur: u8, color: Color32, pct: u8) -> Shadow {
    Shadow { offset: [0, y], blur, spread: 0, color: alpha(color, percent(pct)) }
}

/// The fill, shadows, edge and highlight of `level` in palette `p`.
pub fn elevation(p: Palette, level: Level) -> Elevation {
    // Per level: contact (y, blur, %), key (y, blur, %), edge %, highlight %.
    let (contact, key, edge, highlight) = match (p.dark, level) {
        (false, Level::Card) => ((1, 2, 5), (2, 8, 4), 6, 0),
        (false, Level::Raised) => ((1, 3, 8), (6, 16, 8), 7, 0),
        (false, Level::Overlay) => ((1, 3, 8), (12, 32, 12), 8, 0),
        (false, Level::Modal) => ((2, 6, 8), (24, 56, 18), 8, 0),
        (true, Level::Card) => ((1, 2, 30), (4, 12, 20), 6, 4),
        (true, Level::Raised) => ((1, 2, 40), (8, 20, 30), 8, 6),
        (true, Level::Overlay) => ((1, 3, 45), (12, 32, 40), 9, 7),
        (true, Level::Modal) => ((2, 6, 50), (24, 56, 50), 10, 8),
    };
    let (shade, edge_tint) = if p.dark { (Color32::BLACK, Color32::WHITE) } else { (p.ink, p.ink) };
    let fill = match (p.dark, level) {
        (false, _) => p.panel,
        (true, Level::Card) => rgb(0x24, 0x27, 0x2B),
        (true, Level::Raised) => rgb(0x2A, 0x2D, 0x32),
        (true, Level::Overlay) => rgb(0x2D, 0x31, 0x36),
        (true, Level::Modal) => rgb(0x30, 0x34, 0x39),
    };
    Elevation {
        fill,
        contact: drop_shadow(contact.0, contact.1, shade, contact.2),
        key: drop_shadow(key.0, key.1, shade, key.2),
        edge: Stroke::new(1.0_f32, alpha(edge_tint, percent(edge))),
        highlight: alpha(Color32::WHITE, percent(highlight)),
    }
}

/// The two shadows of `e` around a surface covering `rect`, widest first.
fn shadow_shapes(e: &Elevation, rect: Rect, radius: f32) -> Vec<Shape> {
    [e.key, e.contact].iter().map(|s| Shape::Rect(s.as_shape(rect, radius))).collect()
}

/// The 1 px top highlight, inset by the corner radius so it stays on the straight edge.
fn paint_highlight(painter: &Painter, rect: Rect, radius: f32, e: &Elevation) {
    if e.highlight == Color32::TRANSPARENT {
        return;
    }
    let inset = radius.min(rect.width() * 0.5);
    painter.hline(rect.left() + inset..=rect.right() - inset, rect.top() + 0.5, Stroke::new(1.0_f32, e.highlight));
}

/// Paints a surface covering `rect` at elevation `e` straight onto `painter`: shadows, fill,
/// edge, then highlight. For surfaces that are not laid out as a [`Frame`], such as the minimap.
pub fn paint_elevation(painter: &Painter, rect: Rect, radius: f32, e: &Elevation) {
    for shape in shadow_shapes(e, rect, radius) {
        painter.add(shape);
    }
    painter.rect(rect, radius, e.fill, e.edge, StrokeKind::Inside);
    paint_highlight(painter, rect, radius, e);
}

/// Shows `add` on a surface at `level`: shadows under a filled, edged frame, with the highlight
/// on top. The shadow slot is reserved before the frame so it paints beneath it.
pub fn elevated<R>(
    ui: &mut Ui,
    p: Palette,
    level: Level,
    radius: f32,
    margin: Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let e = elevation(p, level);
    let slot = ui.painter().add(Shape::Noop);
    let inner = Frame::new()
        .fill(e.fill)
        .stroke(e.edge)
        .corner_radius(radius)
        .inner_margin(margin)
        .show(ui, add);
    let rect = inner.response.rect;
    ui.painter().set(slot, Shape::Vec(shadow_shapes(&e, rect, radius)));
    paint_highlight(ui.painter(), rect, radius, &e);
    inner
}

/// WCAG 2.x contrast ratio between two colours (1 to 21). Alpha is ignored; composite with
/// [`over`] first.
#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
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
    let overlay = elevation(p, Level::Overlay);
    visuals.window_stroke = overlay.edge;
    visuals.window_corner_radius = CornerRadius::same(token::RADIUS_DIALOG as u8);
    visuals.menu_corner_radius = CornerRadius::same(token::RADIUS_PANEL as u8);
    visuals.window_shadow = overlay.key;
    visuals.popup_shadow = overlay.key;
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
        // Tooltips wait a beat before appearing and stay quick to follow while moving between
        // controls.
        style.interaction.tooltip_delay = 0.35;
        style.interaction.tooltip_grace_time = 0.6;
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

/// Consistent secondary line used under headings and in footers.
pub fn caption(ui: &mut egui::Ui, p: Palette, text: impl ToString) {
    ui.label(RichText::new(text.to_string()).size(token::FONT_SMALL).color(p.muted));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_rises_with_level() {
        for dark in [false, true] {
            let p = palette(dark);
            let levels = [Level::Card, Level::Raised, Level::Overlay, Level::Modal];
            for pair in levels.windows(2) {
                let (a, b) = (elevation(p, pair[0]), elevation(p, pair[1]));
                assert!(b.key.blur > a.key.blur);
                assert!(b.key.offset[1] >= a.key.offset[1]);
            }
        }
    }

    #[test]
    fn dark_surfaces_lighten_with_level() {
        let p = palette(true);
        let fills: Vec<f32> = [Level::Card, Level::Raised, Level::Overlay, Level::Modal]
            .iter()
            .map(|l| contrast(elevation(p, *l).fill, Color32::BLACK))
            .collect();
        assert!(fills.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn dark_edges_separate_from_backdrop() {
        let p = palette(true);
        for (level, floor) in [(Level::Card, 1.25), (Level::Overlay, 1.4), (Level::Modal, 1.4)] {
            let e = elevation(p, level);
            assert!(contrast(over(e.edge.color, e.fill), p.app) >= floor, "{level:?}");
        }
    }

    #[test]
    fn text_contrast_holds_on_every_fill() {
        for dark in [false, true] {
            let p = palette(dark);
            for level in [Level::Card, Level::Raised, Level::Overlay, Level::Modal] {
                assert!(contrast(p.muted, elevation(p, level).fill) >= 4.5);
            }
        }
    }

    #[test]
    fn contrast_matches_wcag_extremes() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-4);
    }
}
