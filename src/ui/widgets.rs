//! G-Helper style widgets drawn with egui's painter: glowing per-mode buttons, colored section headers,
//! toggle switches, cards and settings rows for the secondary windows

use egui::{Color32, CornerRadius, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::lenovo::fans::{self, FanTableData};
use crate::lighting::zone::Color as ZoneColor;

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    /// Window background (Mica base)
    pub bg: Color32,
    /// Card layer on the background
    pub card: Color32,
    /// Control fill (buttons, tiles)
    pub button: Color32,
    /// Control fill, hovered
    pub button2: Color32,
    /// Hairline card / control border
    pub border: Color32,
    pub text: Color32,
    pub text2: Color32,
    /// Windows accent for selected tiles, toggles and sliders, and the text drawn on it
    pub accent: Color32,
    pub on_accent: Color32,
    pub chart: Color32,
    pub grid: Color32,
}

impl Theme {
    /// G-Helper style palette; LHelper's blue (Balanced) for toggles, sliders and the main action
    pub fn new(dark: bool) -> Theme {
        let accent = rgb(crate::app::COLOR_BALANCED);
        if dark {
            Theme {
                dark,
                bg: Color32::from_rgb(28, 28, 28),
                card: Color32::from_rgb(35, 35, 35),
                button: Color32::from_rgb(46, 46, 46),
                button2: Color32::from_rgb(54, 54, 54),
                border: Color32::from_rgb(58, 58, 58),
                text: Color32::from_rgb(240, 240, 240),
                text2: Color32::from_rgb(160, 160, 160),
                accent,
                on_accent: Color32::BLACK,
                chart: Color32::from_rgb(35, 35, 35),
                grid: Color32::from_rgb(70, 70, 70),
            }
        } else {
            Theme {
                dark,
                bg: Color32::from_rgb(243, 243, 243),
                card: Color32::from_rgb(250, 250, 250),
                button: Color32::from_rgb(252, 252, 252),
                button2: Color32::from_rgb(240, 240, 240),
                border: Color32::from_rgb(218, 218, 218),
                text: Color32::from_rgb(20, 20, 20),
                text2: Color32::from_rgb(100, 100, 100),
                accent,
                on_accent: Color32::BLACK,
                chart: Color32::from_rgb(252, 252, 252),
                grid: Color32::from_rgb(215, 215, 215),
            }
        }
    }

    /// Segoe UI from the Windows fonts folder (regular + semibold as "bold"), like the C# version
    pub fn load_fonts(ctx: &egui::Context) {
        let dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
        let mut fonts = egui::FontDefinitions::default();
        if let Ok(regular) = std::fs::read(dir.join("segoeui.ttf")) {
            fonts.font_data.insert("segoe".into(), std::sync::Arc::new(egui::FontData::from_owned(regular)));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoe".into());
        }
        let bold = std::fs::read(dir.join("seguisb.ttf")).or_else(|_| std::fs::read(dir.join("segoeuib.ttf")));
        if let Ok(bold) = bold {
            fonts.font_data.insert("segoe_bold".into(), std::sync::Arc::new(egui::FontData::from_owned(bold)));
            let mut family = vec!["segoe_bold".to_string()];
            family.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
            fonts.families.insert(egui::FontFamily::Name("bold".into()), family);
        } else {
            let fallback = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
            fonts.families.insert(egui::FontFamily::Name("bold".into()), fallback);
        }
        // Consolas for the monospace labels the RGB controller uses
        if let Ok(mono) = std::fs::read(dir.join("consola.ttf")) {
            fonts.font_data.insert("consolas".into(), std::sync::Arc::new(egui::FontData::from_owned(mono)));
            fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "consolas".into());
        }
        // Segoe Fluent Icons (Windows 11), Segoe MDL2 Assets (Windows 10) for the chevrons and the settings gear
        if let Ok(icons) = std::fs::read(dir.join("SegoeIcons.ttf")).or_else(|_| std::fs::read(dir.join("segmdl2.ttf"))) {
            fonts.font_data.insert("icons".into(), std::sync::Arc::new(egui::FontData::from_owned(icons)));
            fonts.families.insert(egui::FontFamily::Name("icons".into()), vec!["icons".into()]);
        } else {
            let fallback = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
            fonts.families.insert(egui::FontFamily::Name("icons".into()), fallback);
        }
        ctx.set_fonts(fonts);
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.card;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.extreme_bg_color = self.chart;
        visuals.override_text_color = Some(self.text);
        visuals.selection.bg_fill = self.accent;
        visuals.selection.stroke = Stroke::new(1.0, self.on_accent);
        visuals.hyperlink_color = self.accent;
        let radius = CornerRadius::same(5);
        for w in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
            w.corner_radius = radius;
        }
        visuals.widgets.inactive.bg_fill = self.button;
        visuals.widgets.inactive.weak_bg_fill = self.button;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.hovered.bg_fill = self.button2;
        visuals.widgets.hovered.weak_bg_fill = self.button2;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.active.weak_bg_fill = self.button2;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        visuals.slider_trailing_fill = true;
        visuals.handle_shape = egui::style::HandleShape::Circle;
        visuals.menu_corner_radius = CornerRadius::same(8);
        visuals.window_corner_radius = CornerRadius::same(8);
        ctx.set_visuals(visuals);
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(8.0, 7.0);
            s.spacing.button_padding = Vec2::new(12.0, 5.0);
            s.spacing.slider_width = 260.0;
            s.spacing.interact_size.y = 24.0;
        });
    }
}

pub fn icons(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("icons".into()))
}

// Segoe Fluent Icons glyphs
pub const ICON_SETTINGS: &str = "\u{E713}";
pub const ICON_SPEED: &str = "\u{EC4A}";
pub const ICON_GAUGE: &str = "\u{EC48}";
pub const ICON_CHIP: &str = "\u{E950}";
pub const ICON_SCREEN: &str = "\u{E7F4}";
pub const ICON_KEYBOARD: &str = "\u{E765}";
pub const ICON_BATTERY: &str = "\u{EBA7}";
pub const ICON_LEAF: &str = "\u{E8BE}";
pub const ICON_POWER: &str = "\u{E7E8}";
pub const ICON_BOLT: &str = "\u{E945}";
pub const ICON_SLIDERS: &str = "\u{E9E9}";
pub const ICON_THERMO: &str = "\u{E9CA}";
pub const ICON_LAPTOP: &str = "\u{E7F8}";
pub const ICON_SPARKLE: &str = "\u{F4A5}";
pub const ICON_SYNC: &str = "\u{E895}";
pub const ICON_MOON: &str = "\u{E708}";
pub const ICON_BULB: &str = "\u{EA80}";

fn hover_fill(theme: &Theme, base: Color32, response: &Response) -> Color32 {
    let toward = if theme.dark { Color32::WHITE } else { Color32::BLACK };
    if response.is_pointer_button_down_on() {
        blend(base, toward, 0.03)
    } else if response.hovered() {
        blend(base, toward, if theme.dark { 0.05 } else { 0.03 })
    } else {
        base
    }
}

/// A Fluent card: layered fill, hairline border, 8 px corners, full width
pub fn card<R>(ui: &mut Ui, theme: &Theme, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme.card)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Always top-down inside, even when the card sits in a horizontal row
            ui.vertical(add).inner
        })
        .inner
}

/// Card heading: semibold title, muted detail on the right
pub fn card_title(ui: &mut Ui, theme: &Theme, title: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(bold(14.5)).color(theme.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(detail).size(13.0).color(theme.text2));
        });
    });
}

/// G-Helper style mode button: the selected one glows in its own color with a colored border. With an icon
/// (Segoe Fluent Icons glyph) the icon sits above the label, tinted with the button's color when selected.
pub fn mode_button(ui: &mut Ui, theme: &Theme, icon: &str, text: &str, active: bool, accent: Color32, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(7);
    painter.rect_filled(rect, radius, hover_fill(theme, theme.button, &response));
    if active {
        // soft glow fading down from the top (stacked translucent slices), then the accent border
        let slices = 14;
        let inner = rect.shrink(2.0);
        let strength = if theme.dark { 0.06 } else { 0.045 };
        for i in 0..slices {
            let h = inner.height() * 0.7 * (1.0 - i as f32 / slices as f32);
            let r = Rect::from_min_max(inner.min, Pos2::new(inner.max.x, inner.min.y + h));
            painter.rect_filled(r, CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, accent.gamma_multiply(strength));
        }
        painter.rect_stroke(rect.shrink(1.2), radius, Stroke::new(2.4, accent), StrokeKind::Middle);
    } else {
        let border = if response.hovered() { accent.gamma_multiply(0.6) } else { theme.border };
        painter.rect_stroke(rect.shrink(0.5), radius, Stroke::new(1.0, border), StrokeKind::Middle);
    }
    let font = if active { bold(13.5) } else { egui::FontId::proportional(13.5) };
    if icon.is_empty() || size.y < 48.0 {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, theme.text);
    } else {
        let tint = if active { accent } else { theme.text2 };
        painter.text(rect.center() - Vec2::new(0.0, 9.0), egui::Align2::CENTER_CENTER, icon, icons(18.0), tint);
        painter.text(rect.center() + Vec2::new(0.0, 13.0), egui::Align2::CENTER_CENTER, text, font, theme.text);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// G-Helper style section title: icon in the section's current color, bold title, live info on the right
pub fn section_header(ui: &mut Ui, theme: &Theme, icon: &str, title: &str, info: &str, color: Color32) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).font(icons(17.0)).color(color));
        ui.label(RichText::new(title).font(bold(15.0)).color(theme.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(info).size(13.0).color(theme.text2));
        });
    });
}

/// Neutral button with an icon before the label
pub fn icon_button(ui: &mut Ui, theme: &Theme, icon: &str, text: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), hover_fill(theme, theme.button, &response));
    painter.rect_stroke(rect.shrink(0.5), CornerRadius::same(6), Stroke::new(1.0, theme.border), StrokeKind::Middle);
    let font = egui::FontId::proportional(13.5);
    let label = painter.layout_no_wrap(text.to_string(), font, theme.text);
    let gap = if icon.is_empty() { 0.0 } else { 22.0 };
    let start = rect.center().x - (label.size().x + gap) / 2.0;
    if !icon.is_empty() {
        painter.text(Pos2::new(start + 7.0, rect.center().y), egui::Align2::CENTER_CENTER, icon, icons(14.0), theme.text2);
    }
    painter.galley(Pos2::new(start + gap, rect.center().y - label.size().y / 2.0), label, theme.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Standard (neutral) button
pub fn secondary_button(ui: &mut Ui, theme: &Theme, text: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(5), hover_fill(theme, theme.button, &response));
    painter.rect_stroke(rect.shrink(0.5), CornerRadius::same(5), Stroke::new(1.0, theme.border), StrokeKind::Middle);
    let color = if enabled { theme.text } else { theme.text2.gamma_multiply(0.6) };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.5), color);
    response
}

/// Accent (primary) button, one per window for the main action
pub fn primary_button(ui: &mut Ui, theme: &Theme, text: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() { blend(theme.accent, if theme.dark { Color32::BLACK } else { Color32::WHITE }, 0.1) } else { theme.accent };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, bold(13.5), theme.on_accent);
    response
}

/// Windows 11 toggle switch
pub fn toggle(ui: &mut Ui, theme: &Theme, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let painter = ui.painter();
    let radius = CornerRadius::same(10);
    if *on {
        painter.rect_filled(rect, radius, theme.accent);
    } else {
        painter.rect_filled(rect, radius, hover_fill(theme, theme.chart, &response));
        painter.rect_stroke(rect.shrink(0.5), radius, Stroke::new(1.0, theme.text2), StrokeKind::Middle);
    }
    let knob = if response.hovered() { 7.0 } else { 6.0 };
    let x = egui::lerp((rect.min.x + 10.0)..=(rect.max.x - 10.0), t);
    painter.circle_filled(Pos2::new(x, rect.center().y), knob, if *on { theme.on_accent } else { theme.text2 });
    response
}

/// Label (and optional description) on the left, toggle switch on the right
pub fn toggle_row(ui: &mut Ui, theme: &Theme, label: &str, description: &str, on: &mut bool) -> Response {
    let mut response = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 56.0);
            ui.label(RichText::new(label).size(13.5).color(theme.text));
            if !description.is_empty() {
                ui.label(RichText::new(description).size(12.0).color(theme.text2));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            response = Some(toggle(ui, theme, on));
        });
    });
    response.unwrap()
}

/// Thin divider between rows inside a card
pub fn divider(ui: &mut Ui, theme: &Theme) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, if theme.dark { Color32::from_rgb(56, 56, 56) } else { theme.border }));
}

/// The 24 keyboard zones as a rounded strip (the main window's compact preview)
pub fn zone_strip(ui: &mut Ui, theme: &Theme, colors: &[ZoneColor], height: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), theme.chart);
    let w = rect.width() / 24.0;
    for (i, c) in colors.iter().take(24).enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.min.x + i as f32 * w, rect.min.y), Vec2::new(w + 0.5, rect.height()));
        let corners = match i {
            0 => CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 },
            23 => CornerRadius { nw: 0, sw: 0, ne: 4, se: 4 },
            _ => CornerRadius::ZERO,
        };
        painter.rect_filled(r, corners, Color32::from_rgb(c.r, c.g, c.b));
    }
    painter.rect_stroke(rect.shrink(0.5), CornerRadius::same(4), Stroke::new(1.0, theme.border), StrokeKind::Middle);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("bold".into()))
}

pub fn rgb(c: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Lays out `count` equal-width cells in a row and calls `cell` for each with its size
pub fn row(ui: &mut Ui, count: usize, height: f32, mut cell: impl FnMut(&mut Ui, usize, Vec2)) {
    if count == 0 {
        return;
    }
    let gap = 6.0;
    let width = (ui.available_width() - gap * (count as f32 - 1.0)) / count as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for i in 0..count {
            cell(ui, i, Vec2::new(width, height));
        }
    });
}

// ---------------- keyboard visualizer (LOQ-24-zone-RGB-controller style) ----------------

static LAYOUT: once_cell::sync::OnceCell<egui::TextureHandle> = once_cell::sync::OnceCell::new();
static GLOW: std::sync::Mutex<Option<egui::TextureHandle>> = std::sync::Mutex::new(None);

/// Keyboard artwork from LOQ-24-zone-RGB-controller (MIT, Dhruv Goel)
fn layout_texture(ctx: &egui::Context) -> Option<&'static egui::TextureHandle> {
    LAYOUT
        .get_or_try_init(|| {
            let img = image::load_from_memory(include_bytes!("../../assets/layout.png")).map_err(|_| ())?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Ok::<_, ()>(ctx.load_texture("kb_layout", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR))
        })
        .ok()
}

/// Layout aspect ratio (1280 x 480)
pub const KEYBOARD_ASPECT: f32 = 1280.0 / 480.0;

/// Same rendering as the original KeyboardVisualizer: a 24 x 4 glow (alpha 0 / 160 / 160 / 0) stretched with bilinear
/// filtering at 60% opacity, the keyboard artwork on top, and a highlighted column with "Zone NN" on hover.
pub fn keyboard_visualizer(ui: &mut Ui, colors: &[ZoneColor], size: Vec2, background: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let ctx = ui.ctx().clone();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, background);

    let mut pixels = Vec::with_capacity(24 * 4 * 4);
    for row in 0..4 {
        let alpha = if row == 1 || row == 2 { 160 } else { 0 };
        for x in 0..24 {
            let c = colors.get(x).copied().unwrap_or(ZoneColor::BLACK);
            pixels.extend_from_slice(&[c.r, c.g, c.b, alpha]);
        }
    }
    let glow_image = egui::ColorImage::from_rgba_unmultiplied([24, 4], &pixels);
    let mut glow = GLOW.lock().unwrap();
    match glow.as_mut() {
        Some(t) => t.set(glow_image, egui::TextureOptions::LINEAR),
        None => *glow = Some(ctx.load_texture("kb_glow", glow_image, egui::TextureOptions::LINEAR)),
    }
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if let Some(t) = glow.as_ref() {
        painter.image(t.id(), rect, uv, Color32::WHITE.gamma_multiply(0.6));
    }
    drop(glow);
    if let Some(layout) = layout_texture(&ctx) {
        painter.image(layout.id(), rect, uv, Color32::WHITE);
    }

    if let Some(p) = response.hover_pos() {
        let width = rect.width() / 24.0;
        let zone = (((p.x - rect.min.x) / width) as usize).min(23);
        let column = Rect::from_min_size(Pos2::new(rect.min.x + zone as f32 * width, rect.min.y), Vec2::new(width, rect.height()));
        painter.rect_filled(column, CornerRadius::ZERO, Color32::from_white_alpha(20));
        painter.rect_stroke(column, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
        return response.on_hover_text(format!("Zone {:02}", zone + 1));
    }
    response
}

/// Draggable 10-point fan curve. Y = level 0-10 (labelled with the first fan's RPM), X = temperature points.
pub fn fan_curve(ui: &mut Ui, theme: &Theme, table: &mut [i32; 10], data: &[FanTableData], enabled: bool, drag: &mut Option<usize>) -> bool {
    let size = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), theme.chart);

    let plot = Rect::from_min_max(Pos2::new(rect.min.x + 46.0, rect.min.y + 14.0), Pos2::new(rect.max.x - 14.0, rect.max.y - 30.0));
    let primary = data.first();
    let point = |i: usize, level: i32| Pos2::new(plot.min.x + plot.width() * i as f32 / 9.0, plot.max.y - plot.height() * level as f32 / 10.0);
    // Same mapping as LLT's FanCurveControl: level n runs at FanSpeeds[n] (level 1 = slowest running speed, not off)
    let rpm = |d: &FanTableData, level: i32| d.speeds.get(level.max(0) as usize).or(d.speeds.last()).copied().unwrap_or(0);
    let font = egui::FontId::proportional(11.0);

    for level in (0..=10).step_by(2) {
        let y = plot.max.y - plot.height() * level as f32 / 10.0;
        painter.line_segment([Pos2::new(plot.min.x, y), Pos2::new(plot.max.x, y)], Stroke::new(1.0, theme.grid));
        let label = primary.map(|d| rpm(d, level).to_string()).unwrap_or(level.to_string());
        painter.text(Pos2::new(plot.min.x - 6.0, y), egui::Align2::RIGHT_CENTER, label, font.clone(), theme.text2);
    }
    for i in 0..10 {
        let x = plot.min.x + plot.width() * i as f32 / 9.0;
        painter.line_segment([Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)], Stroke::new(1.0, theme.grid));
        let label = primary.map(|d| format!("{}°", d.temps[i])).unwrap_or(format!("P{}", i + 1));
        painter.text(Pos2::new(x, plot.max.y + 6.0), egui::Align2::CENTER_TOP, label, font.clone(), theme.text2);
    }

    let color = if enabled { theme.accent } else { theme.text2 };
    let points: Vec<Pos2> = (0..10).map(|i| point(i, table[i])).collect();

    // filled area under the curve
    let mut poly = vec![Pos2::new(plot.min.x, plot.max.y)];
    poly.extend(points.iter().copied());
    poly.push(Pos2::new(plot.max.x, plot.max.y));
    for w in poly.windows(2).skip(1).take(points.len() - 1) {
        let quad = vec![w[0], w[1], Pos2::new(w[1].x, plot.max.y), Pos2::new(w[0].x, plot.max.y)];
        painter.add(egui::Shape::convex_polygon(quad, color.gamma_multiply(0.18), Stroke::NONE));
    }
    painter.add(egui::Shape::line(points.clone(), Stroke::new(2.5, color)));

    // interaction
    let mut changed = false;
    let minimum = fans::minimum_table();
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let nearest = pointer.filter(|p| p.x >= plot.min.x - 12.0 && p.x <= plot.max.x + 12.0).map(|p| (((p.x - plot.min.x) / plot.width()).clamp(0.0, 1.0) * 9.0).round() as usize);
    if enabled {
        if response.drag_started() || response.clicked() {
            *drag = nearest;
        }
        if let (Some(i), Some(p)) = (*drag, pointer) {
            if response.dragged() || response.clicked() {
                let level = ((((plot.max.y - p.y) / plot.height()).clamp(0.0, 1.0) * 10.0).round() as i32).clamp(minimum[i], 10);
                if table[i] != level {
                    table[i] = level;
                    for j in i + 1..10 {
                        table[j] = table[j].max(level);
                    }
                    for j in (0..i).rev() {
                        table[j] = table[j].min(level).max(minimum[j]);
                    }
                    changed = true;
                }
            }
        }
        if response.drag_stopped() {
            *drag = None;
        }
    }

    let hover = drag.or(nearest.filter(|_| response.hovered()));
    for (i, p) in points.iter().enumerate() {
        let active = Some(i) == hover;
        let r = if active { 7.0 } else { 5.0 };
        painter.circle(*p, r, if active { color } else { theme.chart }, Stroke::new(2.0, color));
    }

    if let Some(i) = hover {
        let mut lines = vec![format!("Point {}: level {}", i + 1, table[i])];
        for d in data {
            lines.push(format!("{:?} {}°C → {} RPM", d.kind, d.temps[i], rpm(d, table[i])));
        }
        let text = lines.join("\n");
        let galley = painter.layout_no_wrap(text, font, theme.text);
        let mut pos = points[i] + Vec2::new(10.0, -galley.size().y - 12.0);
        pos.x = pos.x.min(rect.max.x - galley.size().x - 10.0);
        pos.y = pos.y.max(rect.min.y + 2.0);
        let bg = Rect::from_min_size(pos, galley.size()).expand(5.0);
        painter.rect_filled(bg, CornerRadius::same(4), theme.button);
        painter.rect_stroke(bg, CornerRadius::same(4), Stroke::new(1.0, theme.border), StrokeKind::Inside);
        painter.galley(pos, galley, theme.text);
    }
    changed
}
