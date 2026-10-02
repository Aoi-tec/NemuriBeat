//! egui 標準に無い部品。見た目はデザインシステムのコンポーネントと同じ。

use eframe::egui;
use eframe::egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui,
};

use crate::theme::{card, muted, radius, size, space, text_style, Palette, SANS_SEMIBOLD};

fn font(ui: &Ui, name: &'static str) -> FontId {
    text_style(name).resolve(ui.style())
}

/// Toggle: 36×20 のトラックと 12px のノブ。ON は accent、OFF は surface-sunken に line-strong の枠。
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let p = Palette::of(ui);
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let full = CornerRadius::same(radius::FULL);
        let painter = ui.painter();
        if *on {
            painter.rect_filled(rect, full, p.accent);
        } else {
            painter.rect(
                rect,
                full,
                p.surface_sunken,
                Stroke::new(size::STROKE_HAIRLINE, p.line_strong),
                StrokeKind::Inside,
            );
        }
        let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        painter.circle_filled(
            pos2(x, rect.center().y),
            6.0,
            if *on { p.on_accent } else { p.ink_muted },
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(3.0),
                full,
                Stroke::new(size::STROKE_FOCUS, p.accent),
                StrokeKind::Outside,
            );
        }
    }
    response
}

/// SegmentedControl: 2〜5 個の排他的な選択肢。変わったら true。
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
    small: bool,
) -> bool {
    let p = Palette::of(ui);
    let h = if small {
        size::CONTROL_H_SM
    } else {
        size::CONTROL_H
    };
    let (pad_x, font_id) = if small {
        (space::S2, font(ui, "label"))
    } else {
        (space::S3, TextStyle::Body.resolve(ui.style()))
    };
    let bold = FontId::new(font_id.size, FontFamily::Name(SANS_SEMIBOLD.into()));
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), bold.clone(), p.ink)
                .size()
                .x
                + pad_x * 2.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 2.0 * (options.len() as f32 - 1.0) + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        CornerRadius::same(radius::MD),
        p.surface_sunken,
        Stroke::new(size::STROKE_HAIRLINE, p.line_strong),
        StrokeKind::Inside,
    );
    let mut changed = false;
    let mut x = rect.left() + 2.0;
    for ((option, label), w) in options.iter().zip(widths) {
        let cell = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(w, h - 4.0));
        let response = ui.interact(
            cell,
            ui.id().with(("seg", *label, x as i32)),
            Sense::click(),
        );
        let selected = *value == *option;
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
        let inner = CornerRadius::same(if small { radius::SM } else { radius::MD - 2 });
        if selected {
            painter.rect_filled(cell, inner, p.accent_soft);
        } else if response.hovered() {
            painter.rect_filled(cell, inner, p.hover);
        }
        let (f, color) = if selected {
            (bold.clone(), p.accent_ink)
        } else {
            (font_id.clone(), p.ink_muted)
        };
        painter.text(cell.center(), Align2::CENTER_CENTER, *label, f, color);
        x += w + 2.0;
    }
    changed
}

/// StatusBadge の種類。形（● ◆ ■ ○）と語と色の 3 つで伝える。
#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    Ok,
    Warn,
    Danger,
    Off,
}

/// StatusBadge: 状態を 1〜2 語で示す。
pub fn badge(ui: &mut Ui, status: Status, text: &str) -> Response {
    let p = Palette::of(ui);
    let (ink, fill) = match status {
        Status::Ok => (p.accent_ink, Some(p.accent_soft)),
        Status::Warn => (p.warn, Some(p.warn_soft)),
        Status::Danger => (p.danger, Some(p.danger_soft)),
        Status::Off => (p.ink_muted, None),
    };
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), TextStyle::Small.resolve(ui.style()), ink);
    let (rect, response) = ui.allocate_exact_size(
        vec2(6.0 + 8.0 + 6.0 + galley.size().x + space::S2, 20.0),
        Sense::hover(),
    );
    let painter = ui.painter();
    if let Some(fill) = fill {
        painter.rect_filled(rect, CornerRadius::same(radius::SM), fill);
    }
    dot(ui, p, pos2(rect.left() + 10.0, rect.center().y), status);
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    response
}

/// タイルの中身の最小の高さ。hero のタイルに合わせてある。
pub const TILE_MIN_H: f32 = 112.0;

/// TextField: 高さ control-h の 1 行入力。数値やアドレスは `mono = true`。
pub fn text_field(ui: &mut Ui, value: &mut String, width: f32, mono: bool) -> Response {
    let style = if mono {
        TextStyle::Monospace
    } else {
        TextStyle::Body
    };
    ui.add(
        egui::TextEdit::singleline(value)
            .font(style)
            .desired_width(width)
            .margin(egui::Margin::symmetric(space::S3 as i8, 6)),
    )
}

/// StatTile: ラベル、値（等幅）、単位、比較の 1 行。`hero` は 1 画面に 1 つだけ。
pub fn stat_tile(ui: &mut Ui, label: &str, value: &str, unit: &str, note: &str, hero: bool) {
    stat_tile_parts(ui, label, &[(value, unit)], note, hero);
}

/// 値と単位が複数組ある StatTile（「5 時間 48 分」など）。
pub fn stat_tile_parts(ui: &mut Ui, label: &str, parts: &[(&str, &str)], note: &str, hero: bool) {
    let p = Palette::of(ui);
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(TILE_MIN_H); // 同じ行のタイルの高さを揃える
        ui.spacing_mut().item_spacing.y = space::S1;
        ui.label(
            RichText::new(label)
                .text_style(text_style("label"))
                .color(p.ink_muted),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let style = if hero { "stat-hero" } else { "stat" };
            for (value, unit) in parts {
                let ink = if *value == "—" { p.ink_faint } else { p.ink };
                let value_height = ui
                    .label(
                        RichText::new(*value)
                            .text_style(text_style(style))
                            .color(ink),
                    )
                    .rect
                    .height();
                // 単位は値のベースライン付近に揃える
                let unit_width = ui
                    .painter()
                    .layout_no_wrap((*unit).to_owned(), font(ui, "label"), p.ink_muted)
                    .size()
                    .x;
                ui.allocate_ui_with_layout(
                    vec2(unit_width, value_height),
                    Layout::bottom_up(Align::Min),
                    |ui| {
                        ui.add_space(if hero { 8.0 } else { 4.0 });
                        ui.label(
                            RichText::new(*unit)
                                .text_style(text_style("label"))
                                .color(p.ink_muted),
                        );
                    },
                );
            }
        });
        // 列の中では折り返した行が両端揃えになるので、補足は 1 行に収める
        let note = RichText::new(note)
            .text_style(TextStyle::Small)
            .color(p.ink_muted);
        ui.add(egui::Label::new(note).truncate());
    });
}

/// ProbabilityMeter: Sleep Probability と閾値（0.40 / 0.75）。`None` は判定不能。
pub fn probability_meter(
    ui: &mut Ui,
    probability: Option<f32>,
    awake_below: f32,
    asleep_above: f32,
) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let track = Rect::from_min_size(pos2(rect.left(), rect.top() + 6.0), vec2(rect.width(), 8.0));
    let full = CornerRadius::same(radius::FULL);
    let painter = ui.painter();
    painter.rect_filled(track, full, p.sleep_awake);
    if let Some(v) = probability {
        let w = (track.width() * v.clamp(0.0, 1.0)).max(8.0);
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(w, 8.0)),
            full,
            p.series_sleep,
        );
    }
    let small = TextStyle::Small.resolve(ui.style());
    for t in [awake_below, asleep_above] {
        let x = track.left() + track.width() * t;
        painter.line_segment(
            [pos2(x, track.top() - 4.0), pos2(x, track.bottom() + 4.0)],
            Stroke::new(1.0_f32, p.line_strong),
        );
    }
    for (at, label) in [
        (awake_below / 2.0, "覚醒"),
        ((awake_below + asleep_above) / 2.0, "移行"),
        ((asleep_above + 1.0) / 2.0, "睡眠"),
    ] {
        painter.text(
            pos2(track.left() + track.width() * at, rect.bottom()),
            Align2::CENTER_BOTTOM,
            label,
            small.clone(),
            p.ink_faint,
        );
    }
}

/// Sidebar の項目。選択中は accent-soft の塗りに accent-ink の太字。
pub fn nav_item(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let p = Palette::of(ui);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), size::CONTROL_H), Sense::click());
    let painter = ui.painter();
    let md = CornerRadius::same(radius::MD);
    let (fill, ink, family): (Color32, Color32, FontFamily) = if selected {
        (
            p.accent_soft,
            p.accent_ink,
            FontFamily::Name(SANS_SEMIBOLD.into()),
        )
    } else if response.hovered() {
        (p.hover, p.ink, FontFamily::Proportional)
    } else {
        (Color32::TRANSPARENT, p.ink_muted, FontFamily::Proportional)
    };
    painter.rect_filled(rect, md, fill);
    painter.text(
        pos2(rect.left() + space::S3, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(13.0, family),
        ink,
    );
    response
}

// ---------------------------------------------------------------- 画面の骨組み

fn dot(ui: &Ui, p: &Palette, c: egui::Pos2, status: Status) {
    let painter = ui.painter();
    match status {
        Status::Ok => {
            painter.circle_filled(c, 4.0, p.accent);
        }
        Status::Warn => {
            let d = 4.5;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(0.0, -d),
                    c + vec2(d, 0.0),
                    c + vec2(0.0, d),
                    c + vec2(-d, 0.0),
                ],
                p.warn,
                Stroke::NONE,
            ));
        }
        Status::Danger => {
            painter.rect_filled(
                Rect::from_center_size(c, vec2(8.0, 8.0)),
                CornerRadius::same(1),
                p.danger,
            );
        }
        Status::Off => {
            painter.circle_stroke(c, 3.0, Stroke::new(2.0_f32, p.ink_faint));
        }
    }
}

/// Panel: 見出し付きのカード。`right` は見出しの右に置く 1 つの要素（バッジや補足）。
pub fn panel(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui)) {
    let p = Palette::of(ui);
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(size::CONTROL_H_SM);
            ui.label(RichText::new(title).text_style(text_style("heading")));
            ui.with_layout(Layout::right_to_left(Align::Center), right);
        });
        ui.add_space(space::S1);
        body(ui);
    });
}

/// グラフ 1 段の見出し。左に指標名と単位、右に読み値（ラベルと値）。
pub fn chart_head(ui: &mut Ui, title: &str, unit: &str, read: Option<(String, String)>) {
    let p = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(size::CONTROL_H_SM);
        ui.label(RichText::new(title).text_style(text_style("heading")));
        ui.label(muted(ui, unit));
        if let Some((label, value)) = read {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(value)
                        .text_style(text_style("value"))
                        .color(p.ink),
                );
                ui.label(
                    RichText::new(label)
                        .text_style(TextStyle::Small)
                        .color(p.ink_faint),
                );
            });
        }
    });
}

/// SettingRow: 左に名前と 1 文の説明、右にコントロール 1 つ。行の下に区切り線。
pub fn setting_row(ui: &mut Ui, title: &str, sub: &str, control: impl FnOnce(&mut Ui)) {
    let row = vec2(ui.available_width(), size::ROW_H + space::S2);
    // 右のコントロールを先に置き、残った幅で説明を折り返す
    ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
        control(ui);
        ui.add_space(space::S3);
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(space::S1);
            ui.label(title);
            ui.add(egui::Label::new(muted(ui, sub)).wrap());
            ui.add_space(space::S1);
        });
    });
    ui.separator();
}

/// 変えられない項目の値。コントロールの代わりに右側へ書く。
pub fn info(ui: &mut Ui, text: &str, mono: bool) {
    let p = Palette::of(ui);
    let style = if mono {
        TextStyle::Monospace
    } else {
        TextStyle::Body
    };
    ui.label(RichText::new(text).text_style(style).color(p.ink_muted));
}

/// Notice: その場所に関する但し書きを 1〜2 文で。
pub fn notice(ui: &mut Ui, status: Status, text: &str) {
    let p = Palette::of(ui);
    let (fill, ink) = match status {
        Status::Warn => (p.warn_soft, p.ink),
        Status::Danger => (p.danger_soft, p.ink),
        Status::Ok | Status::Off => (p.hover, p.ink_muted),
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(radius::MD))
        .inner_margin(Margin::symmetric(space::S3 as i8, space::S2 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (mark, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                dot(ui, p, mark.center(), status);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .text_style(text_style("label"))
                            .color(ink),
                    )
                    .wrap(),
                );
            });
        });
}

/// 行の中の「ラベル + 値」。接続中のセンサーの値などに使う。
pub fn key_value(ui: &mut Ui, label: &str, value: &str, unit: &str) {
    let p = Palette::of(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(muted(ui, label));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let ink = if value == "—" { p.ink_faint } else { p.ink };
            ui.label(
                RichText::new(value)
                    .text_style(text_style("value"))
                    .color(ink),
            );
            ui.label(muted(ui, unit));
        });
    });
}

/// SaveBar で押されたボタン。
#[derive(PartialEq)]
pub enum SaveAction {
    None,
    Revert,
    Apply,
}

/// SaveBar: 左に状態、右に「元に戻す」と「適用して保存」。
/// `error` があるときは保存できない理由を出し、適用を無効にする。
pub fn save_bar(ui: &mut Ui, changes: usize, error: Option<&str>) -> SaveAction {
    let p = Palette::of(ui);
    let mut action = SaveAction::None;
    ui.horizontal(|ui| {
        ui.set_min_height(size::CONTROL_H);
        match (error, changes) {
            (Some(e), _) => ui.label(
                RichText::new(e)
                    .text_style(text_style("label"))
                    .color(p.danger),
            ),
            (None, 0) => ui.label(muted(ui, "変更はありません")),
            (None, n) => ui.label(
                RichText::new(format!("未保存の変更が {n} 件あります"))
                    .text_style(text_style("label")),
            ),
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(changes > 0 && error.is_none(), |ui| {
                if crate::theme::primary_button(ui, "適用して保存").clicked() {
                    action = SaveAction::Apply;
                }
            });
            ui.add_enabled_ui(changes > 0, |ui| {
                if ui.add(egui::Button::new("元に戻す").frame(false)).clicked() {
                    action = SaveAction::Revert;
                }
            });
        });
    });
    action
}

/// 表の 1 セル。
pub struct Cell {
    pub text: String,
    /// 列の幅（表の幅に対する割合）。
    pub width: f32,
    /// 右揃えの等幅（数値と時刻）。
    pub numeric: bool,
    /// 左揃えの等幅（OSC アドレスなどの識別子）。
    pub code: bool,
    pub muted: bool,
    /// 注意の色（「中断」など）。
    pub warn: bool,
}

impl Cell {
    pub fn text(text: impl Into<String>, width: f32) -> Self {
        Self {
            text: text.into(),
            width,
            numeric: false,
            code: false,
            muted: false,
            warn: false,
        }
    }
    pub fn number(text: impl Into<String>, width: f32) -> Self {
        Self {
            numeric: true,
            ..Self::text(text, width)
        }
    }
    pub fn code(text: impl Into<String>, width: f32) -> Self {
        Self {
            code: true,
            ..Self::text(text, width)
        }
    }
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }
    pub fn warn(mut self) -> Self {
        self.warn = true;
        self
    }
}

/// 表の 1 行。高さ `row-h`、下に `line` の区切り。`header` は見出し行。
/// 行全体が押せる。選択中は accent-soft、ホバーは hover。
pub fn table_row(
    ui: &mut Ui,
    cells: &[Cell],
    header: bool,
    selected: bool,
    clickable: bool,
) -> Response {
    let p = Palette::of(ui);
    let height = if header { 32.0 } else { size::ROW_H };
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::ZERO, p.accent_soft);
    } else if clickable && response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    let inner = rect.shrink2(vec2(space::S4, 0.0));
    let mut x = inner.left();
    for cell in cells {
        let w = inner.width() * cell.width;
        let (font_id, ink) = if header {
            (font(ui, "label"), p.ink_muted)
        } else if cell.numeric {
            (
                font(ui, "value"),
                if cell.muted { p.ink_faint } else { p.ink },
            )
        } else if cell.code {
            (TextStyle::Monospace.resolve(ui.style()), p.ink)
        } else if cell.warn {
            (TextStyle::Body.resolve(ui.style()), p.warn)
        } else {
            (
                TextStyle::Body.resolve(ui.style()),
                if cell.muted { p.ink_muted } else { p.ink },
            )
        };
        if cell.numeric {
            painter.text(
                pos2(x + w - space::S2, rect.center().y),
                Align2::RIGHT_CENTER,
                &cell.text,
                font_id,
                ink,
            );
        } else {
            painter.text(
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                &cell.text,
                font_id,
                ink,
            );
        }
        x += w;
    }
    painter.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(size::STROKE_HAIRLINE, p.line),
    );
    response
}

/// 凡例の 1 行。色見本（10px の角丸）と語を並べる。
pub fn legend(ui: &mut Ui, items: &[(Color32, &str)]) {
    let p = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
        for (color, text) in items {
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
            let swatch = Rect::from_center_size(rect.center(), vec2(10.0, 10.0));
            ui.painter().rect(
                swatch,
                CornerRadius::same(2),
                *color,
                Stroke::new(1.0_f32, p.line_strong),
                StrokeKind::Inside,
            );
            ui.label(
                RichText::new(*text)
                    .text_style(TextStyle::Small)
                    .color(p.ink_muted),
            );
            ui.add_space(space::S3);
        }
    });
}

/// グラフが描けないときの空状態。何が起きているかと、次に何が起きるかを 2 行で。
pub fn empty_state(ui: &mut Ui, title: &str, body: &str, height: f32) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(radius::MD), p.plot_gap);
    painter.text(
        rect.center() - vec2(0.0, 10.0),
        Align2::CENTER_CENTER,
        title,
        font(ui, "body-strong"),
        p.ink,
    );
    painter.text(
        rect.center() + vec2(0.0, 12.0),
        Align2::CENTER_CENTER,
        body,
        font(ui, "label"),
        p.ink_muted,
    );
}
