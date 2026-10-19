use egui::{
     pos2, vec2, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, Frame,
    Margin, Pos2, Rect, RichText, Sense, Sides, Stroke, Ui, Visuals,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavIcon {
    Overview,
    Devices,
    Activity,
    Versions,
    Settings,
    Folder,
    SunMoon,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg_sidebar: Color32,
    pub bg_canvas: Color32,
    pub bg_card: Color32,
    pub bg_row_hover: Color32,
    pub bg_input: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    // Signature ChatGPT / Claude / Antigravity high-contrast monochrome primary pill
    pub primary_bg: Color32,
    pub primary_fg: Color32,
    pub secondary_bg: Color32,
    pub secondary_fg: Color32,
    // Semantic accents (soft & refined, never neon)
    pub accent_blue: Color32,
    pub online_dot: Color32,
    pub online_bg: Color32,
    pub warning_dot: Color32,
    pub warning_bg: Color32,
    pub danger_dot: Color32,
    pub danger_bg: Color32,
}

impl Palette {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            // Antigravity / ChatGPT / Cursor Obsidian Dark Mode (Default)
            ThemeMode::Dark => Self {
                bg_sidebar: Color32::from_rgb(17, 17, 19),       // #111113 Deep sidebar
                bg_canvas: Color32::from_rgb(22, 22, 25),        // #161619 Main workspace canvas
                bg_card: Color32::from_rgb(28, 28, 32),          // #1C1C20 Elevated surface card
                bg_row_hover: Color32::from_rgb(36, 36, 41),     // #242429 Subtle interactive hover
                bg_input: Color32::from_rgb(32, 32, 37),         // #202025 Prompt-bar input fill
                border: Color32::from_rgb(44, 44, 51),           // #2C2C33 Crisp 1px card border
                border_subtle: Color32::from_rgb(36, 36, 42),    // #24242A Internal row divider
                text_primary: Color32::from_rgb(244, 244, 246),  // #F4F4F6 Crisp off-white
                text_secondary: Color32::from_rgb(161, 161, 170),// #A1A1AA Zinc 400
                text_muted: Color32::from_rgb(113, 113, 122),    // #71717A Zinc 500
                primary_bg: Color32::from_rgb(244, 244, 246),    // #F4F4F6 Signature white pill button
                primary_fg: Color32::from_rgb(17, 17, 19),       // #111113 Deep black text on white pill
                secondary_bg: Color32::from_rgb(39, 39, 45),     // #27272D Soft charcoal button
                secondary_fg: Color32::from_rgb(228, 228, 231),  // #E4E4E7
                accent_blue: Color32::from_rgb(96, 165, 250),    // #60A5FA Soft sky accent
                online_dot: Color32::from_rgb(52, 211, 153),     // #34D399 Emerald 400
                online_bg: Color32::from_rgb(16, 46, 37),        // Soft dark emerald tint
                warning_dot: Color32::from_rgb(251, 191, 36),    // #FBBF24 Amber 400
                warning_bg: Color32::from_rgb(54, 38, 14),       // Soft dark amber tint
                danger_dot: Color32::from_rgb(248, 113, 113),    // #F87171 Red 400
                danger_bg: Color32::from_rgb(58, 22, 24),        // Soft dark red tint
            },
            // Claude / ChatGPT Warm Alabaster Light Mode
            ThemeMode::Light => Self {
                bg_sidebar: Color32::from_rgb(243, 243, 240),    // #F3F3F0 Warm stone sidebar
                bg_canvas: Color32::from_rgb(250, 250, 248),     // #FAFAF8 Clean paper canvas
                bg_card: Color32::from_rgb(255, 255, 255),       // #FFFFFF Pure white card
                bg_row_hover: Color32::from_rgb(244, 244, 241),  // #F4F4F1
                bg_input: Color32::from_rgb(255, 255, 255),      // #FFFFFF
                border: Color32::from_rgb(228, 228, 224),        // #E4E4E0 Soft hairline border
                border_subtle: Color32::from_rgb(238, 238, 235), // #EEEEEB Internal divider
                text_primary: Color32::from_rgb(24, 24, 27),     // #18181B Deep charcoal ink
                text_secondary: Color32::from_rgb(100, 100, 108),// #64646C
                text_muted: Color32::from_rgb(142, 142, 150),    // #8E8E96
                primary_bg: Color32::from_rgb(24, 24, 27),       // #18181B Signature black pill button
                primary_fg: Color32::from_rgb(250, 250, 250),    // #FAFAFA White text
                secondary_bg: Color32::from_rgb(242, 242, 239),  // #F2F2EF
                secondary_fg: Color32::from_rgb(39, 39, 42),     // #27272A
                accent_blue: Color32::from_rgb(37, 99, 235),     // #2563EB
                online_dot: Color32::from_rgb(22, 163, 74),      // #16A34A
                online_bg: Color32::from_rgb(220, 252, 231),     // #DCFCE7
                warning_dot: Color32::from_rgb(217, 119, 6),     // #D97706
                warning_bg: Color32::from_rgb(254, 243, 199),    // #FEF3C7
                danger_dot: Color32::from_rgb(220, 38, 38),      // #DC2626
                danger_bg: Color32::from_rgb(254, 226, 226),     // #FEE2E2
            },
        }
    }
}

pub fn init_fonts(ctx: &Context) {
    let candidate_paths = [
        "/System/Library/Fonts/SFNS.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/System/Library/Fonts/HelveticaNeue.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];

    for path in candidate_paths {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = FontDefinitions::default();
            fonts.font_data.insert(
                "vn_sys_font".to_owned(),
                Arc::new(FontData::from_owned(bytes)),
            );
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.insert(0, "vn_sys_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                family.push("vn_sys_font".to_owned());
            }
            ctx.set_fonts(fonts);
            break;
        }
    }
}

pub fn apply_theme(ctx: &Context, mode: ThemeMode) {
    let p = Palette::for_mode(mode);
    let mut visuals = match mode {
        ThemeMode::Dark => Visuals::dark(),
        ThemeMode::Light => Visuals::light(),
    };

    visuals.panel_fill = p.bg_canvas;
    visuals.window_fill = p.bg_card;
    visuals.faint_bg_color = p.bg_row_hover;
    visuals.extreme_bg_color = p.bg_input;
    visuals.override_text_color = Some(p.text_primary);

    visuals.widgets.noninteractive.bg_fill = p.bg_card;
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(10);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);

    visuals.widgets.inactive.bg_fill = p.secondary_bg;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);

    visuals.widgets.hovered.bg_fill = p.bg_row_hover;
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.text_muted);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text_primary);

    visuals.widgets.active.bg_fill = p.primary_bg;
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.primary_fg);

    visuals.selection.bg_fill = p.secondary_bg;
    visuals.selection.stroke = Stroke::new(1.0_f32, p.text_primary);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    ctx.set_style(style);
}

/// Khung Card đơn lớp chuẩn Antigravity / Claude (không lồng nhiều khung viền chồng chéo)
pub fn card_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.bg_card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(18, 16))
}

/// Khung Prompt / Command Bar bo tròn kiểu ChatGPT / Antigravity
pub fn prompt_bar_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.bg_input)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 8))
}

/// Đường kẻ phân tách mỏng 1px giữa các dòng bên trong Card (thay cho việc bọc sub_card_frame quanh mỗi dòng)
pub fn hairline_divider(ui: &mut Ui, p: &Palette) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0_f32, p.border_subtle),
    );
    ui.add_space(6.0);
}

pub fn split_row(
    ui: &mut Ui,
    add_left: impl FnOnce(&mut Ui),
    add_right: impl FnOnce(&mut Ui),
) {
    Sides::new().show(ui, add_left, add_right);
}

/// Nút chính (Primary Pill) kiểu ChatGPT / Claude / Antigravity:
/// Nền Trắng Sáng - Chữ Đen trong Dark Mode (hoặc Nền Đen - Chữ Trắng trong Light Mode)
pub fn primary_button(ui: &mut Ui, p: &Palette, label: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(p.primary_fg)
            .strong()
            .size(12.5),
    )
    .fill(p.primary_bg)
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(8))
    .min_size(vec2(0.0, 30.0));
    ui.add(btn)
}

/// Nút phụ (Secondary Ghost/Surface Pill) kiểu Antigravity
pub fn secondary_button(ui: &mut Ui, p: &Palette, label: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(p.secondary_fg)
            .strong()
            .size(12.5),
    )
    .fill(p.secondary_bg)
    .stroke(Stroke::new(1.0_f32, p.border))
    .corner_radius(CornerRadius::same(8))
    .min_size(vec2(0.0, 30.0));
    ui.add(btn)
}

/// Nút nhỏ gọn (Inline Chip Action) dùng cho "Sao chép"
pub fn compact_button(ui: &mut Ui, p: &Palette, label: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(p.text_secondary)
            .size(11.5),
    )
    .fill(p.secondary_bg)
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(6))
    .min_size(vec2(0.0, 22.0));
    ui.add(btn)
}

pub fn danger_button(ui: &mut Ui, p: &Palette, label: &str) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(p.danger_dot)
            .strong()
            .size(12.5),
    )
    .fill(p.danger_bg)
    .stroke(Stroke::new(1.0_f32, p.danger_dot.gamma_multiply(0.4)))
    .corner_radius(CornerRadius::same(8))
    .min_size(vec2(0.0, 30.0));
    ui.add(btn)
}

/// Status Pill tối giản với chấm tròn vector (không viền thô)
pub fn status_pill(ui: &mut Ui, bg: Color32, fg: Color32, text: &str) {
    Frame::new()
        .fill(bg)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.0, fg);
                ui.label(RichText::new(text).color(fg).strong().size(11.5));
            });
        });
}

/// Vẽ các biểu tượng hình học (Vector Icons) sắc nét bằng `egui::Painter`
pub fn paint_nav_icon(ui: &Ui, rect: Rect, icon: NavIcon, color: Color32) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.5_f32, color);
    let c = rect.center();

    match icon {
        NavIcon::Overview => {
            // 2x2 Bento Grid icon (Dashboard)
            let s = 5.0;
            let gap = 1.5;
            let tl = Rect::from_min_size(pos2(c.x - s - gap * 0.5, c.y - s - gap * 0.5), vec2(s, s));
            let tr = Rect::from_min_size(pos2(c.x + gap * 0.5, c.y - s - gap * 0.5), vec2(s, s));
            let bl = Rect::from_min_size(pos2(c.x - s - gap * 0.5, c.y + gap * 0.5), vec2(s, s));
            let br = Rect::from_min_size(pos2(c.x + gap * 0.5, c.y + gap * 0.5), vec2(s, s));
            for r in [tl, tr, bl, br] {
                painter.rect_stroke(r, CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            }
        }
        NavIcon::Devices => {
            // Monitor / Laptop Node icon
            let screen = Rect::from_center_size(pos2(c.x, c.y - 1.5), vec2(12.0, 8.0));
            painter.rect_stroke(screen, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [pos2(c.x - 7.5, c.y + 4.5), pos2(c.x + 7.5, c.y + 4.5)],
                stroke,
            );
        }
        NavIcon::Activity => {
            // Pulse / Waveform icon (Antigravity activity stream)
            let pts = [
                pos2(c.x - 6.5, c.y),
                pos2(c.x - 3.0, c.y),
                pos2(c.x - 1.2, c.y - 5.0),
                pos2(c.x + 1.5, c.y + 5.0),
                pos2(c.x + 3.2, c.y),
                pos2(c.x + 6.5, c.y),
            ];
            for w in pts.windows(2) {
                painter.line_segment([w[0], w[1]], stroke);
            }
        }
        NavIcon::Versions => {
            // Clock / History icon
            painter.circle_stroke(c, 6.0, stroke);
            painter.line_segment([c, pos2(c.x, c.y - 3.5)], stroke);
            painter.line_segment([c, pos2(c.x + 3.0, c.y + 1.5)], stroke);
        }
        NavIcon::Settings => {
            // Sliders icon
            for (dy, knob_dx) in [(-4.0_f32, -2.0_f32), (0.0, 2.5), (4.0, -1.0)] {
                let y = c.y + dy;
                painter.line_segment([pos2(c.x - 6.0, y), pos2(c.x + 6.0, y)], stroke);
                painter.circle_filled(pos2(c.x + knob_dx, y), 2.0, color);
            }
        }
        NavIcon::Folder => {
            // Minimalist Folder outline
            let body = Rect::from_center_size(pos2(c.x, c.y + 1.0), vec2(13.0, 9.0));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [pos2(body.left() + 1.0, body.top() - 2.0), pos2(body.left() + 5.5, body.top() - 2.0)],
                stroke,
            );
        }
        NavIcon::SunMoon => {
            // Half-filled contrast circle (Theme toggle icon)
            painter.circle_stroke(c, 5.5, stroke);
            painter.circle_filled(Pos2::new(c.x + 1.5, c.y), 2.5, color);
        }
    }
}

/// Nút điều hướng Sidebar kiểu Antigravity / ChatGPT / Claude (Ghost item không viền, có Vector Icon bên trái & Badge bên phải)
pub fn sidebar_nav_item(
    ui: &mut Ui,
    p: &Palette,
    icon: NavIcon,
    label: &str,
    badge: Option<usize>,
    is_active: bool,
) -> bool {
    let desired_size = vec2(ui.available_width(), 34.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

    if ui.is_rect_visible(rect) {
        let is_hovered = response.hovered();
        if is_active || is_hovered {
            let fill = if is_active {
                p.bg_row_hover
            } else {
                p.bg_row_hover.gamma_multiply(0.55)
            };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), fill);
        }

        let fg = if is_active {
            p.text_primary
        } else if is_hovered {
            p.text_primary
        } else {
            p.text_secondary
        };

        let icon_rect = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
        paint_nav_icon(ui, icon_rect, icon, fg);

        let font_id = egui::FontId::proportional(13.0);
        ui.painter().text(
            pos2(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font_id,
            fg,
        );

        if let Some(count) = badge {
            let badge_text = count.to_string();
            let badge_rect = Rect::from_center_size(
                pos2(rect.right() - 18.0, rect.center().y),
                vec2(22.0, 18.0),
            );
            ui.painter().rect_filled(badge_rect, CornerRadius::same(9), p.secondary_bg);
            ui.painter().text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                badge_text,
                egui::FontId::proportional(11.0),
                p.text_secondary,
            );
        }
    }

    response.clicked()
}
