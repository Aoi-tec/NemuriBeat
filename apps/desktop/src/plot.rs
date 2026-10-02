//! egui_plot をデザインシステムどおりに組むためのヘルパー。

use std::ops::RangeInclusive;

use eframe::egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Rect, Sense, Stroke,
    StrokeKind, Ui,
};
use egui_plot::{
    Bar, BarChart, HLine, Line, Plot, PlotPoints, PlotResponse, Points, Polygon, VLine,
};

use crate::theme::{radius, size, Palette, MONO_MEDIUM};

/// Tracking = true が続いた区間ごとの点列 `[時刻, 値]`。区間の切れ目が「未計測」。
pub type Runs = [Vec<[f64; 2]>];

// ---------------------------------------------------------------- 縦軸

/// 1-2-5 系列の「きりのいい」刻み。
pub fn nice_step(raw: f64) -> f64 {
    let raw = raw.abs().max(1e-9);
    let pow = 10f64.powf(raw.log10().floor());
    let m = raw / pow;
    pow * if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }
}

/// 目盛りの値。常に 3〜6 本。範囲が広がれば刻みが粗く、狭まれば細かくなる。
fn nice_values(lo: f64, hi: f64) -> (Vec<f64>, f64) {
    let step = nice_step((hi - lo) / 6.0);
    let mut out = Vec::new();
    let mut v = (lo / step - 1e-6).ceil() * step;
    while v <= hi + step * 1e-6 {
        out.push(v);
        v += step;
    }
    (out, step)
}

/// 縦軸の決め方。
#[derive(Clone, Copy)]
pub enum YAxis {
    /// 見えている時間窓の最小・最大に合わせて伸縮する。
    /// `min_span` より狭くは引き延ばさない（心拍数なら 10 bpm）。`floor` は下限（HRV の 0 など）。
    Fit { min_span: f64, floor: Option<f64> },
    /// 下端を 0 に固定し、上端だけ合わせる（棒グラフ）。
    FromZero,
    /// 固定。Sleep Probability の 0〜1 など。
    Fixed(f64, f64),
}

/// いま見えている時間窓 `x` の最小・最大に縦軸を合わせる。
/// 上下に 12% の余白を足し、端を目盛りの刻みの 1/5 に丸める。
/// 窓の最小・最大がその単位をまたいだときだけ軸が動くので、毎拍は揺れない。
pub fn fit_y(
    runs: &Runs,
    x: &RangeInclusive<f64>,
    min_span: f64,
    floor: Option<f64>,
) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for [t, v] in runs.iter().flatten() {
        if x.contains(t) {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    if lo > hi {
        return None; // 窓の中にデータが無い。前の範囲を保つ
    }
    if hi - lo < min_span {
        let mid = (lo + hi) / 2.0;
        lo = mid - min_span / 2.0;
        hi = mid + min_span / 2.0;
    }
    let pad = (hi - lo) * 0.12;
    let snap = nice_step((hi - lo + 2.0 * pad) / 6.0) / 5.0;
    let mut y0 = ((lo - pad) / snap).floor() * snap;
    let y1 = ((hi + pad) / snap).ceil() * snap;
    if let Some(f) = floor {
        y0 = y0.max(f);
    }
    Some((y0, y1))
}

/// `x` に最も近いデータ点。カーソルを点にスナップさせる。
pub fn nearest(runs: &Runs, x: f64) -> Option<[f64; 2]> {
    runs.iter()
        .flatten()
        .min_by(|a, b| (a[0] - x).abs().total_cmp(&(b[0] - x).abs()))
        .copied()
}

// ---------------------------------------------------------------- 1 段ぶんのグラフ

/// 時系列グラフ 1 段の中身。
pub struct Row<'a> {
    pub id: &'static str,
    pub name: &'a str,
    /// 系列色（`p.series_hr` など）。
    pub color: Color32,
    pub runs: &'a Runs,
    pub y: YAxis,
    /// 縦軸の目盛りを自分で決めるとき（Sleep Probability の 0 / 0.40 / 0.75 / 1 など）。
    pub ticks: Option<&'a [f64]>,
    /// 目盛りとタグの小数桁。
    pub decimals: usize,
    pub height: f32,
    /// 系列色を wash で敷く（Sleep Probability だけ）。
    pub wash: bool,
    /// 折れ線ではなく棒で描く（History の推定睡眠時間）。
    pub bars: bool,
    /// 右端が「いま」のグラフ。現在値のタグと水平線を出す。
    pub now_tag: bool,
    /// 基準値のタグ（安静時心拍の Baseline など）。
    pub reference: Option<f64>,
    /// 閾値線（0.40 / 0.75）。
    pub thresholds: &'a [f64],
    /// Baseline の帯（値の範囲）。
    pub band: Option<(f64, f64)>,
    /// Tracking = false の区間。
    pub gaps: &'a [RangeInclusive<f64>],
    /// 横軸の目盛りを出すか。積み重ねでは一番下の段だけ true。
    pub show_x_axis: bool,
}

/// 時間窓の扱い。
pub enum Window {
    /// 固定の窓（Realtime の「直近 5 分」など）。ドラッグもズームもさせない。
    Fixed(RangeInclusive<f64>),
    /// 横にドラッグ、Ctrl + ホイールで横ズーム、ダブルクリックで戻る（一晩のグラフ）。
    Free,
}

fn hover_key(group: Id) -> Id {
    group.with("hover-time")
}

/// 同じ `group` のどこかの段でホバーしている時刻。見出しの読み値にも使う。
pub fn hovered_time(ui: &Ui, group: Id) -> Option<f64> {
    ui.data(|d| d.get_temp::<(Id, f64)>(hover_key(group)))
        .map(|(_, t)| t)
}

fn rect_points(x: &RangeInclusive<f64>, y: &RangeInclusive<f64>) -> PlotPoints<'static> {
    vec![
        [*x.start(), *y.start()],
        [*x.end(), *y.start()],
        [*x.end(), *y.end()],
        [*x.start(), *y.end()],
    ]
    .into()
}

/// 横軸の書式。値を文字にする関数と、目盛りに使ってよい刻みの候補。
#[derive(Clone, Copy)]
pub struct XAxis {
    pub label: fn(f64) -> String,
    pub steps: &'static [f64],
}

/// その日の 0:00 からの分。目盛りは 5 分〜4 時間。
pub const CLOCK: XAxis = XAxis {
    label: clock,
    steps: &[5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 180.0, 240.0],
};
/// 今日を 0 とした日数（昨日が -1）。目盛りは 1 日〜1 年。
pub const DAYS: XAxis = XAxis {
    label: day,
    steps: &[1.0, 2.0, 7.0, 14.0, 30.0, 90.0, 365.0],
};
/// 現在からの秒（右端が 0）。目盛りは 5 秒〜15 分。
pub const AGO: XAxis = XAxis {
    label: ago,
    steps: &[5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0],
};

/// 目盛りと値タグの文字（type トークンの `tick`）。
fn tick_font() -> FontId {
    FontId::new(11.0, FontFamily::Monospace)
}

/// 時系列グラフを 1 段描く。`group` が同じ段は横軸とカーソルを共有する。
pub fn row(ui: &mut Ui, group: Id, window: &Window, x_axis: XAxis, r: &Row<'_>) {
    let p = Palette::of(ui);
    let plot_id = ui.id().with(r.id);
    let hover = hovered_time(ui, group);
    let free = matches!(window, Window::Free);
    let decimals = r.decimals;
    let ticks: Option<Vec<f64>> = r.ticks.map(<[f64]>::to_vec);
    // 右側に縦軸の列（plot-axis-w）を空けておき、目盛りと値タグは自前で描く
    let full_width = ui.available_width();
    ui.set_min_width(full_width); // 縦軸の列まで含めて Panel の幅にする
    let plot_width = full_width - size::PLOT_AXIS_W;

    let plot = Plot::new(r.id)
        .width(plot_width)
        .height(r.height)
        // 地は Panel の surface をそのまま見せる。枠も背景も敷かない
        .show_background(false)
        // グリッドは自前で水平線だけを引く（色を plot-grid に固定するため）
        .show_grid(false)
        // egui_plot の軸は使わない（文字の大きさと色を tick に揃えられないため）
        .show_axes([false, false])
        .set_margin_fraction(vec2(0.0, 0.0))
        // 縦は自動で合わせるので、手では動かさせない。ホイールはページのスクロールに譲る
        .allow_drag([free, false])
        .allow_zoom([free, false])
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(free)
        .link_axis(group, [true, false])
        // 既定のホバー表示は使わず、下で自前のカーソルとタグを描く
        .show_x(false)
        .show_y(false);

    let now = r.runs.last().and_then(|run| run.last()).copied();
    let response = plot.show(ui, |plot| {
        // --- 範囲: 横は窓、縦は見えているデータに合わせる
        if let Window::Fixed(x) = window {
            plot.set_plot_bounds_x(x.clone());
        }
        let visible = match window {
            Window::Fixed(x) => x.clone(),
            Window::Free => plot.plot_bounds().range_x(),
        };
        let (y0, y1) = match r.y {
            YAxis::Fixed(a, b) => (a, b),
            YAxis::Fit { min_span, floor } => fit_y(r.runs, &visible, min_span, floor)
                .unwrap_or_else(|| {
                    let b = plot.plot_bounds().range_y();
                    (*b.start(), *b.end())
                }),
            YAxis::FromZero => (
                0.0,
                fit_y(r.runs, &visible, 1.0, None).map_or(1.0, |(_, y1)| y1.max(1.0)),
            ),
        };
        plot.set_plot_bounds_y(y0..=y1);
        let wide = *visible.start() - 1e9..=*visible.end() + 1e9;

        // --- 背景: 未計測の帯、Baseline の帯、水平グリッド、閾値
        for g in r.gaps {
            plot.polygon(
                Polygon::new("未計測", rect_points(g, &(y0..=y1)))
                    .fill_color(p.plot_gap)
                    .stroke(Stroke::NONE)
                    .allow_hover(false),
            );
        }
        if let Some((lo, hi)) = r.band {
            plot.polygon(
                Polygon::new("Baseline", rect_points(&wide, &(lo..=hi)))
                    .fill_color(p.series_neutral.gamma_multiply(size::WASH))
                    .stroke(Stroke::NONE)
                    .allow_hover(false),
            );
        }
        let grid = ticks.clone().unwrap_or_else(|| nice_values(y0, y1).0);
        for &v in &grid {
            plot.hline(
                HLine::new("", v)
                    .stroke(Stroke::new(size::STROKE_HAIRLINE, p.plot_grid))
                    .allow_hover(false),
            );
        }
        for &t in r.thresholds {
            plot.hline(
                HLine::new("", t)
                    .stroke(Stroke::new(size::STROKE_HAIRLINE, p.line_strong))
                    .allow_hover(false),
            );
        }

        // --- データ: 区間ごとに別の Line にして、未計測をまたいで結ばない
        if let (true, Some([_, v])) = (r.now_tag, now) {
            plot.hline(
                HLine::new("", v)
                    .stroke(Stroke::new(
                        size::STROKE_HAIRLINE,
                        r.color.gamma_multiply(0.5),
                    ))
                    .allow_hover(false),
            );
        }
        let snapped = hover.and_then(|t| nearest(r.runs, t));
        if r.bars {
            // 棒はスロットの 6 割。ホバー中は、その棒以外を薄くする
            let bars = r
                .runs
                .iter()
                .flatten()
                .map(|&[t, v]| {
                    let dim = snapped.is_some_and(|s| s[0] != t);
                    let fill = if dim {
                        r.color.gamma_multiply(0.4)
                    } else {
                        r.color
                    };
                    Bar::new(t, v).fill(fill).stroke(Stroke::NONE)
                })
                .collect();
            plot.bar_chart(
                BarChart::new(r.name, bars)
                    .color(r.color)
                    .width(0.6)
                    .allow_hover(false),
            );
        } else {
            for run in r.runs {
                let width = if run.len() > 1000 {
                    size::STROKE_SERIES_DENSE
                } else {
                    size::STROKE_SERIES
                };
                let mut line = Line::new(r.name, run.clone())
                    .color(r.color)
                    .width(width)
                    .allow_hover(false);
                if r.wash {
                    line = line.fill(y0 as f32).fill_alpha(size::WASH);
                }
                plot.line(line);
            }
        }
        if let (true, Some(pt)) = (r.now_tag, now) {
            plot.points(
                Points::new("", vec![pt])
                    .color(r.color)
                    .radius(4.0_f32)
                    .allow_hover(false),
            );
        }

        // --- カーソル: 最も近い点にスナップした縦線・横線と点
        if let Some([t, v]) = snapped {
            let cursor = Stroke::new(size::STROKE_HAIRLINE, p.line_strong);
            if !r.bars {
                plot.vline(VLine::new("", t).stroke(cursor).allow_hover(false));
                plot.points(
                    Points::new("", vec![[t, v]])
                        .color(r.color)
                        .radius(4.0_f32)
                        .allow_hover(false),
                );
            }
            plot.hline(HLine::new("", v).stroke(cursor).allow_hover(false));
        }
        (
            plot.pointer_coordinate().map(|c| c.x),
            snapped,
            grid,
            visible,
        )
    });

    // --- ホバー中の時刻を同じ group の全段で共有する
    let (pointer_x, snapped, grid, visible) = response.inner.clone();
    ui.data_mut(|d| {
        let key = hover_key(group);
        match pointer_x.filter(|_| response.response.hovered()) {
            Some(t) => {
                d.insert_temp(key, (plot_id, t));
            }
            None => {
                if d.get_temp::<(Id, f64)>(key)
                    .is_some_and(|(owner, _)| owner == plot_id)
                {
                    d.remove::<(Id, f64)>(key);
                }
            }
        }
    });

    // --- 値タグ。重なるときは カーソル > 現在値 > 基準値 の順で 1 つだけ残す
    let frame = *response.transform.frame();
    let screen_y = |v: f64| response.transform.position_from_point_y(v);
    let clear =
        |v: f64, other: Option<f64>| other.is_none_or(|o| (screen_y(o) - screen_y(v)).abs() > 16.0);
    let cursor_v = snapped.map(|s| s[1]);
    let now_v = now
        .filter(|_| r.now_tag)
        .map(|n| n[1])
        .filter(|&v| clear(v, cursor_v));
    let reference_v = r
        .reference
        .filter(|&v| clear(v, cursor_v) && clear(v, now_v));

    // --- 縦軸の目盛り。値タグと重なるものは描かない
    let tagged: Vec<f32> = [reference_v, now_v, cursor_v]
        .into_iter()
        .flatten()
        .map(screen_y)
        .collect();
    for v in grid {
        let sy = screen_y(v);
        if tagged.iter().all(|t| (t - sy).abs() > 16.0) {
            ui.painter().text(
                pos2(frame.right() + 8.0, sy),
                Align2::LEFT_CENTER,
                format!("{:.*}", decimals, v),
                tick_font(),
                p.ink_faint,
            );
        }
    }

    for (value, kind) in [
        (reference_v, Tag::Reference),
        (now_v, Tag::Now),
        (cursor_v, Tag::Cursor),
    ] {
        if let Some(v) = value {
            axis_tag(ui, p, &response, v, &format!("{:.*}", decimals, v), kind);
        }
    }

    // --- 横軸の目盛りと時刻タグ。積み重ねでは一番下の段だけ
    if r.show_x_axis {
        let (strip, _) = ui.allocate_exact_size(vec2(plot_width, 20.0), Sense::hover());
        let (lo, hi) = (*visible.start(), *visible.end());
        let max_ticks = ((plot_width / 80.0).floor() as usize).clamp(2, 8);
        let step = x_axis
            .steps
            .iter()
            .copied()
            .find(|s| (hi - lo) / s <= max_ticks as f64)
            .unwrap_or(*x_axis.steps.last().unwrap_or(&1.0));
        let tag_x = snapped.map(|s| response.transform.position_from_point_x(s[0]));
        let mut t = (lo / step).ceil() * step;
        while t <= hi + step * 1e-6 {
            let sx = response.transform.position_from_point_x(t);
            let align = if sx < frame.left() + 16.0 {
                Align2::LEFT_CENTER
            } else if sx > frame.right() - 16.0 {
                Align2::RIGHT_CENTER
            } else {
                Align2::CENTER_CENTER
            };
            if tag_x.is_none_or(|x| (x - sx).abs() > 46.0) {
                ui.painter().text(
                    pos2(sx, strip.center().y),
                    align,
                    (x_axis.label)(t),
                    tick_font(),
                    p.ink_faint,
                );
            }
            t += step;
        }
        if let Some([t, _]) = snapped {
            time_tag(ui, p, &response, t, &(x_axis.label)(t));
        }
    }
}

// ---------------------------------------------------------------- 値タグ

/// 値タグの種類。
pub enum Tag {
    /// 現在値。`ink` の塗りに `bg` の文字。1 つのグラフに 1 つ。
    Now,
    /// カーソル位置の値。`surface-raised` に `line-strong` の枠。
    Cursor,
    /// 基準値（Baseline など）。枠だけ、文字は `ink-muted`。
    Reference,
}

fn paint_tag(ui: &Ui, p: &Palette, rect: Rect, text: &str, kind: Tag, align: Align2) {
    let (fill, ink, stroke) = match kind {
        Tag::Now => (p.ink, p.bg, Stroke::NONE),
        Tag::Cursor => (
            p.surface_raised,
            p.ink,
            Stroke::new(size::STROKE_HAIRLINE, p.line_strong),
        ),
        Tag::Reference => (
            p.surface,
            p.ink_muted,
            Stroke::new(size::STROKE_HAIRLINE, p.line_strong),
        ),
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(radius::SM),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    let anchor = if align == Align2::LEFT_CENTER {
        rect.left_center() + vec2(6.0, 0.0)
    } else {
        rect.center()
    };
    painter.text(
        anchor,
        align,
        text,
        FontId::new(11.0, FontFamily::Name(MONO_MEDIUM.into())),
        ink,
    );
}

/// 縦軸の列に、値 `y` の高さでタグを描く。`Plot::show` の後に呼ぶ。
pub fn axis_tag<R>(ui: &Ui, p: &Palette, plot: &PlotResponse<R>, y: f64, text: &str, kind: Tag) {
    let frame = *plot.transform.frame();
    let sy = plot.transform.position_from_point_y(y);
    if sy < frame.top() || sy > frame.bottom() {
        return; // 表示範囲の外にあるタグは出さない
    }
    let rect = Rect::from_min_size(
        pos2(frame.right() + 2.0, sy - 9.0),
        vec2(size::PLOT_AXIS_W - 4.0, 18.0),
    );
    paint_tag(ui, p, rect, text, kind, Align2::LEFT_CENTER);
}

/// 横軸の下に、時刻 `x` のタグを描く。
pub fn time_tag<R>(ui: &Ui, p: &Palette, plot: &PlotResponse<R>, x: f64, text: &str) {
    let frame = *plot.transform.frame();
    let sx = plot.transform.position_from_point_x(x);
    let w = 12.0 + 7.0 * text.chars().count() as f32;
    let left = (sx - w / 2.0).clamp(frame.left(), frame.right() - w);
    let rect = Rect::from_min_size(pos2(left, frame.bottom() + 1.0), vec2(w, 18.0));
    paint_tag(ui, p, rect, text, Tag::Cursor, Align2::CENTER_CENTER);
}

// ---------------------------------------------------------------- 横軸の書式

/// その日の 0:00 からの分 → `HH:MM`。
pub fn clock(minutes: f64) -> String {
    let m = (minutes.round() as i64).rem_euclid(1440);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// 今日を 0 とした日数 → `M/D`。
pub fn day(days: f64) -> String {
    let date = chrono::Local::now().date_naive() + chrono::Duration::days(days.round() as i64);
    date.format("%-m/%-d").to_string()
}

/// 現在からの秒（右端が 0）→ `-M:SS`。
pub fn ago(seconds: f64) -> String {
    let s = (-seconds).round() as i64;
    if s <= 0 {
        "現在".to_owned()
    } else {
        format!("-{}:{:02}", s / 60, s % 60)
    }
}

// ---------------------------------------------------------------- 睡眠帯

/// 睡眠 / 移行 / 覚醒の帯。`group` と `window` を下のグラフと同じにすると、一緒に動く。
/// `segments` は区間と色。区間の間は 2px 空け、角は radius-sm で丸める。
pub fn band(ui: &mut Ui, group: Id, window: &Window, segments: &[(RangeInclusive<f64>, Color32)]) {
    let p = Palette::of(ui);
    let plot_id = ui.id().with("band");
    let hover = hovered_time(ui, group);
    let free = matches!(window, Window::Free);
    let full_width = ui.available_width();
    ui.set_min_width(full_width);
    let lo = segments
        .iter()
        .map(|s| *s.0.start())
        .fold(f64::INFINITY, f64::min);
    let hi = segments
        .iter()
        .map(|s| *s.0.end())
        .fold(f64::NEG_INFINITY, f64::max);

    let response = Plot::new("band")
        .width(full_width - size::PLOT_AXIS_W)
        .height(32.0)
        .show_background(false)
        .show_grid(false)
        .show_axes([false, false])
        .set_margin_fraction(vec2(0.0, 0.0))
        .allow_drag([free, false])
        .allow_zoom([free, false])
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(free)
        .link_axis(group, [true, false])
        .show_x(false)
        .show_y(false)
        .show(ui, |plot| {
            if let Window::Fixed(x) = window {
                plot.set_plot_bounds_x(x.clone());
            }
            plot.set_plot_bounds_y(0.0..=1.0);
            if lo < hi {
                // 見えない線を 1 本置いて、横軸の初期範囲を記録の範囲に合わせる
                let extent = Line::new("", vec![[lo, 0.5], [hi, 0.5]])
                    .color(Color32::TRANSPARENT)
                    .allow_hover(false);
                plot.line(extent);
            }
            if let Some(t) = hover {
                let cursor = Stroke::new(size::STROKE_HAIRLINE, p.line_strong);
                plot.vline(VLine::new("", t).stroke(cursor).allow_hover(false));
            }
            plot.pointer_coordinate().map(|c| c.x)
        });

    ui.data_mut(|d| {
        let key = hover_key(group);
        match response.inner.filter(|_| response.response.hovered()) {
            Some(t) => {
                d.insert_temp(key, (plot_id, t));
            }
            None => {
                if d.get_temp::<(Id, f64)>(key)
                    .is_some_and(|(owner, _)| owner == plot_id)
                {
                    d.remove::<(Id, f64)>(key);
                }
            }
        }
    });

    let frame = *response.transform.frame();
    let painter = ui.painter().with_clip_rect(frame);
    for (range, color) in segments {
        let x0 = response.transform.position_from_point_x(*range.start());
        let x1 = response.transform.position_from_point_x(*range.end());
        let rect = Rect::from_min_max(
            pos2(x0, frame.center().y - 8.0),
            pos2((x1 - 2.0).max(x0 + 1.0), frame.center().y + 8.0),
        );
        painter.rect_filled(rect, CornerRadius::same(radius::SM), *color);
    }
}
