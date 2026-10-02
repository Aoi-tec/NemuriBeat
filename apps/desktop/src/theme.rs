//! NemuriBeat のテーマ。値はすべてデザインシステムの tokens.json と同じ。
//! ここ以外に色・寸法を直書きしない。
// トークンは一式で定義しておく。まだ画面で使っていないものも残す。
#![allow(dead_code)]

use std::sync::Arc;

use eframe::egui;
use eframe::egui::style::{HandleShape, ScrollStyle};
use eframe::egui::{
    vec2, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Margin, RichText, Shadow, Stroke, Style, TextStyle, Theme, Ui, Visuals,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// color トークン。フィールド名はトークン名の `-` を `_` にしたもの。
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub surface_sunken: Color32,
    pub hover: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub ink: Color32,
    pub ink_muted: Color32,
    pub ink_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub accent_ink: Color32,
    pub on_accent: Color32,
    pub series_sleep: Color32,
    pub series_hr: Color32,
    pub series_hrv: Color32,
    pub series_neutral: Color32,
    pub sleep_transition: Color32,
    pub sleep_awake: Color32,
    pub plot_grid: Color32,
    pub plot_gap: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
}

pub const DARK: Palette = Palette {
    bg: rgb(0x0d, 0x10, 0x12),
    surface: rgb(0x14, 0x18, 0x1b),
    surface_raised: rgb(0x1b, 0x20, 0x24),
    surface_sunken: rgb(0x09, 0x0b, 0x0d),
    hover: rgb(0x20, 0x26, 0x2b),
    line: rgb(0x26, 0x2d, 0x33),
    line_strong: rgb(0x67, 0x73, 0x7d),
    ink: rgb(0xe9, 0xed, 0xf0),
    ink_muted: rgb(0xa3, 0xae, 0xb7),
    ink_faint: rgb(0x87, 0x93, 0x9d),
    accent: rgb(0x3e, 0xcf, 0xb4),
    accent_hover: rgb(0x62, 0xda, 0xc3),
    accent_soft: rgb(0x12, 0x30, 0x2b),
    accent_ink: rgb(0x56, 0xd8, 0xbf),
    on_accent: rgb(0x06, 0x22, 0x1d),
    series_sleep: rgb(0x39, 0x87, 0xe5),
    series_hr: rgb(0xd9, 0x59, 0x26),
    series_hrv: rgb(0xc9, 0x6d, 0xad),
    series_neutral: rgb(0x7f, 0x8b, 0x95),
    sleep_transition: rgb(0x1f, 0x4f, 0x8f),
    sleep_awake: rgb(0x2b, 0x33, 0x39),
    plot_grid: rgb(0x20, 0x26, 0x2b),
    plot_gap: rgb(0x1b, 0x20, 0x24),
    warn: rgb(0xe8, 0xb2, 0x3a),
    warn_soft: rgb(0x33, 0x27, 0x0c),
    danger: rgb(0xff, 0x7a, 0x7a),
    danger_soft: rgb(0x3a, 0x17, 0x19),
};

pub const LIGHT: Palette = Palette {
    bg: rgb(0xf3, 0xf5, 0xf6),
    surface: rgb(0xff, 0xff, 0xff),
    surface_raised: rgb(0xff, 0xff, 0xff),
    surface_sunken: rgb(0xea, 0xee, 0xf0),
    hover: rgb(0xe9, 0xed, 0xf0),
    line: rgb(0xdd, 0xe3, 0xe7),
    line_strong: rgb(0x7a, 0x87, 0x91),
    ink: rgb(0x12, 0x17, 0x1b),
    ink_muted: rgb(0x51, 0x5d, 0x67),
    ink_faint: rgb(0x5e, 0x6a, 0x74),
    accent: rgb(0x0b, 0x7a, 0x68),
    accent_hover: rgb(0x09, 0x65, 0x55),
    accent_soft: rgb(0xd8, 0xf1, 0xec),
    accent_ink: rgb(0x0a, 0x6b, 0x5b),
    on_accent: rgb(0xff, 0xff, 0xff),
    series_sleep: rgb(0x2a, 0x78, 0xd6),
    series_hr: rgb(0xeb, 0x68, 0x34),
    series_hrv: rgb(0x95, 0x3d, 0x7c),
    series_neutral: rgb(0x7a, 0x87, 0x91),
    sleep_transition: rgb(0x86, 0xb6, 0xef),
    sleep_awake: rgb(0xd3, 0xda, 0xdf),
    plot_grid: rgb(0xe9, 0xed, 0xf0),
    plot_gap: rgb(0xee, 0xf1, 0xf3),
    warn: rgb(0x8a, 0x5a, 0x00),
    warn_soft: rgb(0xfb, 0xef, 0xd2),
    danger: rgb(0xc0, 0x30, 0x3a),
    danger_soft: rgb(0xfb, 0xe1, 0xe2),
};

impl Palette {
    /// 今のテーマのパレット。ウィジェットの中ではこれだけを使う。
    pub fn of(ui: &Ui) -> &'static Palette {
        if ui.visuals().dark_mode {
            &DARK
        } else {
            &LIGHT
        }
    }
}

/// spacing トークン（px）。
pub mod space {
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S6: f32 = 24.0;
    pub const S8: f32 = 32.0;
}

/// radius トークン。egui の CornerRadius は u8 なので radius-full は 255。
pub mod radius {
    pub const SM: u8 = 4;
    pub const MD: u8 = 8;
    pub const LG: u8 = 12;
    pub const FULL: u8 = 255;
}

/// size / stroke / opacity トークン。
pub mod size {
    pub const CONTROL_H: f32 = 32.0;
    pub const CONTROL_H_SM: f32 = 24.0;
    pub const ROW_H: f32 = 40.0;
    pub const SIDEBAR_W: f32 = 208.0;
    pub const PLOT_H_SM: f32 = 72.0;
    pub const PLOT_H: f32 = 144.0;
    pub const PLOT_H_LG: f32 = 224.0;
    pub const PLOT_AXIS_W: f32 = 48.0;
    pub const STROKE_HAIRLINE: f32 = 1.0;
    pub const STROKE_SERIES: f32 = 2.0;
    pub const STROKE_SERIES_DENSE: f32 = 1.5;
    pub const STROKE_FOCUS: f32 = 2.0;
    pub const WASH: f32 = 0.10;
    pub const DISABLED: f32 = 0.45;
}

/// egui にはフォントウェイトがないので、ウェイトごとにファミリーを分ける。
pub const SANS_SEMIBOLD: &str = "sans-semibold";
pub const MONO_MEDIUM: &str = "mono-medium";

/// type トークンのうち egui 標準の TextStyle に無いもの。
pub fn text_style(name: &'static str) -> TextStyle {
    TextStyle::Name(name.into())
}

/// 4 本のフォントファイル。`bundled_fonts()` が同梱の IBM Plex を返す。
pub struct FontFiles {
    pub sans_regular: &'static [u8],
    pub sans_semibold: &'static [u8],
    pub mono_regular: &'static [u8],
    pub mono_medium: &'static [u8],
}

/// assets/fonts に同梱した IBM Plex（SIL Open Font License）。
pub fn bundled_fonts() -> FontFiles {
    FontFiles {
        sans_regular: include_bytes!("../assets/fonts/IBMPlexSansJP-Regular.ttf"),
        sans_semibold: include_bytes!("../assets/fonts/IBMPlexSansJP-SemiBold.ttf"),
        mono_regular: include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
        mono_medium: include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    }
}

/// 起動時に 1 回だけ呼ぶ。
pub fn install(ctx: &Context, fonts: FontFiles) {
    install_fonts(ctx, fonts);
    ctx.set_style_of(Theme::Dark, style(&DARK, true));
    ctx.set_style_of(Theme::Light, style(&LIGHT, false));
}

fn install_fonts(ctx: &Context, files: FontFiles) {
    let mut defs = FontDefinitions::default();
    for (name, bytes) in [
        ("plex-sans-jp-regular", files.sans_regular),
        ("plex-sans-jp-semibold", files.sans_semibold),
        ("plex-mono-regular", files.mono_regular),
        ("plex-mono-medium", files.mono_medium),
    ] {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    // egui 既定のフォントは絵文字などのフォールバックとして後ろに残す。
    let fallback = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let chain = |first: &[&str]| -> Vec<String> {
        first
            .iter()
            .map(|s| (*s).to_owned())
            .chain(fallback.iter().cloned())
            .collect()
    };
    // 等幅フォントに無い和文は Plex Sans JP へ落とす。
    defs.families
        .insert(FontFamily::Proportional, chain(&["plex-sans-jp-regular"]));
    defs.families.insert(
        FontFamily::Monospace,
        chain(&["plex-mono-regular", "plex-sans-jp-regular"]),
    );
    defs.families.insert(
        FontFamily::Name(SANS_SEMIBOLD.into()),
        chain(&["plex-sans-jp-semibold"]),
    );
    defs.families.insert(
        FontFamily::Name(MONO_MEDIUM.into()),
        chain(&["plex-mono-medium", "plex-sans-jp-semibold"]),
    );
    ctx.set_fonts(defs);
}

fn style(p: &Palette, dark: bool) -> Style {
    let semibold = || FontFamily::Name(SANS_SEMIBOLD.into());
    let mono_medium = || FontFamily::Name(MONO_MEDIUM.into());

    // ---- type ----
    let text_styles = [
        (TextStyle::Heading, FontId::new(20.0, semibold())), // title
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)), // body
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ), // caption
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ), // code
        (text_style("heading"), FontId::new(15.0, semibold())),
        (text_style("body-strong"), FontId::new(13.0, semibold())),
        (
            text_style("label"),
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (text_style("stat-hero"), FontId::new(48.0, mono_medium())),
        (text_style("stat"), FontId::new(28.0, mono_medium())),
        (text_style("value"), FontId::new(13.0, mono_medium())),
        (text_style("tick"), FontId::new(11.0, FontFamily::Monospace)),
    ]
    .into();
    let mut s = Style {
        text_styles,
        ..Default::default()
    };

    // ---- spacing / size ----
    let sp = &mut s.spacing;
    sp.item_spacing = vec2(space::S2, space::S2);
    sp.button_padding = vec2(space::S3, 6.0); // 6 + 20 (行高) + 6 = control-h
    sp.interact_size = vec2(size::ROW_H, size::CONTROL_H);
    sp.window_margin = Margin::same(space::S4 as i8);
    sp.menu_margin = Margin::same(space::S1 as i8);
    sp.indent = space::S4;
    sp.icon_width = 16.0;
    sp.icon_width_inner = 8.0;
    sp.icon_spacing = space::S2;
    sp.slider_width = 160.0;
    sp.scroll = ScrollStyle::floating(); // 幅を取らないスクロールバー

    // ---- interaction ----
    s.interaction.selectable_labels = false; // ラベルを I ビームで選択させない
    s.interaction.tooltip_delay = 0.3;
    s.animation_time = 0.12;

    // ---- colour ----
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: if dark {
            Color32::from_black_alpha(128)
        } else {
            Color32::from_rgba_unmultiplied(18, 23, 27, 36)
        },
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface_raised;
    v.window_stroke = Stroke::new(size::STROKE_HAIRLINE, p.line);
    v.window_corner_radius = CornerRadius::same(radius::LG);
    v.menu_corner_radius = CornerRadius::same(radius::MD);
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.extreme_bg_color = p.surface_sunken;
    v.text_edit_bg_color = Some(p.surface_sunken);
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.surface_sunken;
    v.hyperlink_color = p.accent_ink;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.danger;
    v.weak_text_color = Some(p.ink_faint);
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(size::STROKE_HAIRLINE, p.accent_ink);
    v.text_cursor.stroke = Stroke::new(size::STROKE_FOCUS, p.accent);
    v.disabled_alpha = size::DISABLED;
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.slider_trailing_fill = true;
    v.handle_shape = HandleShape::Circle;

    let md = CornerRadius::same(radius::MD);
    let w = &mut v.widgets;
    // ラベル、区切り線、枠
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(size::STROKE_HAIRLINE, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.noninteractive.corner_radius = md;
    // 通常のボタン・入力・チェックボックス
    w.inactive.bg_fill = p.surface_sunken;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(size::STROKE_HAIRLINE, p.line_strong);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.inactive.corner_radius = md;
    w.inactive.expansion = 0.0;
    // ホバー: 塗りだけ変える。膨らませない（レイアウトが揺れる）
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(size::STROKE_HAIRLINE, p.line_strong);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, p.ink);
    w.hovered.corner_radius = md;
    w.hovered.expansion = 0.0;
    // 押下・キーボードフォーカス: accent の 2px リング
    w.active.bg_fill = p.hover;
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(size::STROKE_FOCUS, p.accent);
    w.active.fg_stroke = Stroke::new(1.5_f32, p.ink);
    w.active.corner_radius = md;
    w.active.expansion = 0.0;
    // 開いているコンボボックスやメニュー
    w.open.bg_fill = p.surface_raised;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(size::STROKE_HAIRLINE, p.line_strong);
    w.open.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.open.corner_radius = md;
    w.open.expansion = 0.0;

    s.visuals = v;
    s
}

// ---------------------------------------------------------------- 部品

/// Panel: カードとグラフパネルの枠。影は付けない。
pub fn card(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::STROKE_HAIRLINE, p.line))
        .corner_radius(CornerRadius::same(radius::LG))
        .inner_margin(Margin::same(space::S4 as i8))
}

/// Button（プライマリ）。1 画面に 1 つまで。
pub fn primary_button(ui: &mut Ui, label: &str) -> egui::Response {
    let p = Palette::of(ui);
    let text = RichText::new(label)
        .color(p.on_accent)
        .family(FontFamily::Name(SANS_SEMIBOLD.into()));
    ui.add(egui::Button::new(text).fill(p.accent).stroke(Stroke::NONE))
}

/// 副次的な文字（ink-muted）。
pub fn muted(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text)
        .text_style(text_style("label"))
        .color(Palette::of(ui).ink_muted)
}

/// 動く数値。必ず等幅で組む。
pub fn stat(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text)
        .text_style(text_style("stat"))
        .color(Palette::of(ui).ink)
}
