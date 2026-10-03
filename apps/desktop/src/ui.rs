//! 画面。見た目と言葉づかいはデザインシステム（NemuriBeat）に合わせる。
//! 色・寸法は theme.rs、部品は widgets.rs、グラフは plot.rs にあり、ここでは並べるだけにする。

use crate::plot::{self, Row, Window, YAxis};
use crate::runtime::{Command, View};
use crate::theme::{self, muted, size, space, text_style, Palette};
use crate::widgets::{self, Cell, SaveAction, Status};
use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, TextStyle, Ui};
use physio_core::RealtimeCore;
use shared_types::*;
use std::ops::RangeInclusive;
use std::sync::{atomic::Ordering, Arc, Mutex};
use tokio::sync::mpsc;

type Runs = Vec<Vec<[f64; 2]>>;

/// sleep-engine のヒステリシスと同じ閾値。グラフの閾値線と睡眠帯の色分けに使う。
const AWAKE_BELOW: f64 = 0.40;
const ASLEEP_ABOVE: f64 = 0.75;
/// 信号品質がこれを下回ると Tracking が解除される（%）。
const QUALITY_FLOOR: f64 = 80.0;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Dashboard,
    Sleep,
    Realtime,
    History,
    Osc,
    Sensors,
    Settings,
}

const PAGES: [(Page, &str); 7] = [
    (Page::Dashboard, "Dashboard"),
    (Page::Sleep, "Sleep"),
    (Page::Realtime, "Realtime"),
    (Page::History, "History"),
    (Page::Osc, "VRChat OSC"),
    (Page::Sensors, "Sensors"),
    (Page::Settings, "Settings"),
];

pub struct Desktop {
    core: RealtimeCore,
    health: Arc<storage::Health>,
    view: Arc<Mutex<View>>,
    commands: mpsc::Sender<Command>,
    /// 編集中の設定。「適用して保存」で `saved` になる。
    settings: Settings,
    brand: Option<egui::TextureHandle>,
    hyperate_key: String,
    resident: crate::resident::Resident,
    /// いま動いている設定。
    saved: Settings,
    osc_port: String,
    api_port: String,
    page: Page,
    demo: bool,
    notice: String,
    data_dir: String,
    /// Realtime と Dashboard の時間幅（秒）。
    window_secs: f64,
    /// History の表示期間（日）。0 は全期間。
    range_days: u32,
    /// Sleep で開いているセッション。None は「記録中、なければ最新」。
    session: Option<String>,
    /// 1 分足の読み込みを頼んであるセッション。
    requested: Option<String>,
    reloaded_at: f64,
    theme: egui::ThemePreference,
}

impl Desktop {
    pub fn new(
        core: RealtimeCore,
        health: Arc<storage::Health>,
        view: Arc<Mutex<View>>,
        commands: mpsc::Sender<Command>,
        settings: Settings,
        demo: bool,
        data_dir: String,
    ) -> Self {
        Self {
            core,
            health,
            view,
            commands,
            osc_port: settings.osc_port.to_string(),
            api_port: settings.api_port.to_string(),
            saved: settings.clone(),
            settings,
            brand: None,
            hyperate_key: String::new(),
            resident: Default::default(),
            page: Page::Dashboard,
            demo,
            notice: String::new(),
            data_dir,
            window_secs: 300.0,
            range_days: 30,
            session: None,
            requested: None,
            reloaded_at: 0.0,
            theme: egui::ThemePreference::Dark,
        }
    }

    pub fn start_hidden(&mut self, value: bool) {
        self.resident.start_hidden(value);
    }

    fn command(&mut self, c: Command) -> bool {
        let sent = self.commands.try_send(c).is_ok();
        self.notice = if sent {
            String::new()
        } else {
            "処理が混み合っています。もう一度お試しください".into()
        };
        sent
    }

    /// 1 フレームぶんの画面を描く。
    pub fn draw(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let p = if ctx.style().visuals.dark_mode {
            &theme::DARK
        } else {
            &theme::LIGHT
        };
        let s = self.core.latest();
        let shared = self.view.clone();
        let v = shared.lock().unwrap();
        self.keep_session_loaded(ctx, &v);

        egui::SidePanel::left("navigation")
            .exact_width(size::SIDEBAR_W)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(space::S2 as i8, space::S3 as i8)),
            )
            .show(ctx, |ui| self.sidebar(ui, &s, &v));
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(space::S6 as i8, 6)),
            )
            .show(ctx, |ui| self.status_bar(ui, &v));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin::same(space::S6 as i8))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = space::S2;
                                match self.page {
                                    Page::Dashboard => self.dashboard(ui, &s, &v),
                                    Page::Sleep => {
                                        if !s.rr_available {widgets::notice(ui,Status::Off,"RR未取得のため睡眠計測は停止中です。擬似睡眠はRealtime、保存済み記録はHistoryで確認できます。");}
                                        ui.add_enabled_ui(s.rr_available,|ui|self.sleep(ui,&v));
                                    },
                                    Page::Realtime => self.realtime(ui, &s, &v),
                                    Page::History => self.history(ui, &v),
                                    Page::Osc => self.osc(ui, &s, &v),
                                    Page::Sensors => self.sensors(ui, &s, &v),
                                    Page::Settings => self.settings_page(ui, &v),
                                }
                            });
                    });
            });
    }

    /// 表示するセッション: 選んだもの、なければ記録中、なければ最新。
    fn shown_session<'a>(&self, v: &'a View) -> Option<&'a SleepSession> {
        let by_id = |id: &String| v.sessions.iter().find(|t| &t.session_id == id);
        self.session
            .as_ref()
            .and_then(by_id)
            .or_else(|| v.sessions.iter().find(|t| day_is_active(v, &t.session_id)))
            .or(v.sessions.first())
    }

    /// 表示するセッションの 1 分足を読み込んでおく。記録中のものは 60 秒ごとに読み直す。
    fn keep_session_loaded(&mut self, ctx: &egui::Context, v: &View) {
        let Some(id) = self.shown_session(v).map(|t| t.session_id.clone()) else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let recording = day_is_active(v, &id);
        let stale = recording && now - self.reloaded_at > 60.0;
        if (self.requested.as_ref() != Some(&id) || stale)
            && self.command(Command::SelectSession(id.clone()))
        {
            self.requested = Some(id);
            self.reloaded_at = now;
        }
    }

    // ------------------------------------------------------------ 枠

    fn sidebar(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.add_space(space::S3);
            let logo = self.brand.get_or_insert_with(|| {
                let rgba = image::load_from_memory(include_bytes!(
                    "../assets/branding/nemuribeat-appicon-256.png"
                ))
                .expect("bundled logo")
                .into_rgba8();
                ui.ctx().load_texture(
                    "nemuribeat-logo",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
            ui.image((logo.id(), egui::vec2(28.0, 28.0)));
            ui.label(RichText::new("NemuriBeat").text_style(text_style("body-strong")));
        });
        ui.add_space(space::S3);
        for (page, name) in PAGES {
            let enabled = page != Page::Sleep || s.rr_available;
            if ui
                .add_enabled_ui(enabled, |ui| widgets::nav_item(ui, self.page == page, name))
                .inner
                .clicked()
            {
                self.page = page;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = space::S2;
            ui.horizontal(|ui| {
                ui.add_space(space::S1);
                if s.sensor_connected {
                    widgets::badge(ui, Status::Ok, &sensor_name(s, v));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(space::S2);
                        ui.label(
                            RichText::new(opt(s.heart_rate, 0)).text_style(text_style("value")),
                        );
                    });
                } else {
                    widgets::badge(ui, Status::Off, "センサー未接続");
                }
            });
            if self.demo {
                widgets::badge(ui, Status::Warn, "DEMO · 合成データ");
            }
            ui.separator();
        });
    }

    fn status_bar(&mut self, ui: &mut Ui, v: &View) {
        let error = self.health.error.lock().unwrap().clone();
        let stats = [
            ("取りこぼし", self.health.dropped.load(Ordering::Relaxed)),
            ("待ち", self.health.pending.load(Ordering::Relaxed)),
            ("保存", self.health.written.load(Ordering::Relaxed)),
        ];
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for (label, n) in stats {
                ui.label(RichText::new(grouped(n)).text_style(text_style("value")));
                ui.label(muted(ui, label));
                ui.add_space(space::S2);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let message = match &error {
                    Some(e) => {
                        widgets::badge(ui, Status::Danger, "保存を再試行中");
                        format!("{e}。記録はメモリに保持しています")
                    }
                    None if !self.notice.is_empty() => {
                        widgets::badge(ui, Status::Warn, "未送信");
                        self.notice.clone()
                    }
                    None => localize(&v.message),
                };
                ui.add(egui::Label::new(muted(ui, message)).truncate());
            });
        });
    }

    // ------------------------------------------------------------ Dashboard

    fn dashboard(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        let p = Palette::of(ui);
        page_head(ui, "Dashboard", |ui| {
            let (status, text) = osc_state(&v.osc_status);
            widgets::badge(ui, status, &format!("VRChat OSC {text}"));
            if s.sensor_connected {
                widgets::badge(ui, Status::Ok, "センサー 接続中");
            } else {
                widgets::badge(ui, Status::Off, "センサー 未接続");
            }
        });
        let session = self.shown_session(v).cloned();
        ui.columns(4, |c| {
            let baseline = match v.baseline.resting_hr {
                Some(b) => format!("Baseline {b:.0} bpm"),
                None => format!("Baseline 計測中（{} / 30 分）", v.baseline.sample_minutes),
            };
            widgets::stat_tile(
                &mut c[0],
                "現在の心拍",
                &opt(s.heart_rate, 0),
                "bpm",
                &baseline,
                true,
            );
            if s.rr_available {
                let rmssd = format!("RMSSD {} ms", opt(s.rmssd_fast, 0));
                widgets::stat_tile(
                    &mut c[1],
                    "RR Interval",
                    &opt(s.last_rr_ms, 0),
                    "ms",
                    &rmssd,
                    false,
                );
            } else {
                widgets::stat_tile(
                    &mut c[1],
                    "入力モード",
                    if s.sensor_connected {
                        "HRのみ"
                    } else {
                        "未接続"
                    },
                    "",
                    "擬似睡眠 · 睡眠記録なし",
                    false,
                );
            }
            probability_tile(&mut c[2], s);
            match &session {
                Some(t) => {
                    let (h, m) = hours_minutes(t.estimated_sleep_minutes);
                    let label = if t.ended_at.is_none() && v.active.is_some() {
                        "記録中のセッション"
                    } else {
                        "前回の睡眠"
                    };
                    let note = format!(
                        "入眠 {} · 起床 {}",
                        time_opt(t.estimated_sleep_at),
                        time_opt(t.final_wake_at)
                    );
                    widgets::stat_tile_parts(
                        &mut c[3],
                        label,
                        &[(&h, "時間"), (&m, "分")],
                        &note,
                        false,
                    );
                }
                None => widgets::stat_tile(
                    &mut c[3],
                    "前回の睡眠",
                    "—",
                    "",
                    "まだ記録がありません",
                    false,
                ),
            }
        });
        ui.add_space(space::S2);
        split(
            ui,
            0.66,
            |ui| {
                let window = self.window_secs;
                let mut picked = window;
                widgets::panel(
                    ui,
                    "心拍数",
                    |ui| {
                        window_picker(ui, &mut picked);
                    },
                    |ui| {
                        let hr =
                            realtime_runs(v, window, |s| connected(s, s.heart_rate.map(f64::from)));
                        if hr.is_empty() {
                            disconnected_state(ui, size::PLOT_H_LG);
                            return;
                        }
                        let group = ui.id().with("dashboard");
                        plot::row(
                            ui,
                            group,
                            &Window::Fixed(-window..=0.0),
                            plot::AGO,
                            &Row {
                                reference: v.baseline.resting_hr,
                                height: size::PLOT_H_LG,
                                show_x_axis: true,
                                now_tag: true,
                                ..line(
                                    "hr",
                                    "心拍数",
                                    p.series_hr,
                                    &hr,
                                    YAxis::Fit {
                                        min_span: 10.0,
                                        floor: None,
                                    },
                                )
                            },
                        );
                        tag_legend(ui, p, true, v.baseline.resting_hr.is_some());
                    },
                );
                self.window_secs = picked;
            },
            |ui| {
                let target = format!("{}:{}", self.saved.osc_host, self.saved.osc_port);
                widgets::panel(
                    ui,
                    "VRChat OSC",
                    |ui| widgets::info(ui, &target, true),
                    |ui| {
                        let rows = osc_rows(s, &self.saved);
                        if rows.is_empty() {
                            ui.label(muted(ui, "出力するパラメーターがありません"));
                        }
                        let wanted = [
                            "Heartrate/Value",
                            "Heartrate/Normalised",
                            "Sleep/Tracking",
                            "Sleep/Probability",
                        ];
                        for (name, _, value) in
                            rows.iter().filter(|r| wanted.contains(&r.0.as_str()))
                        {
                            widgets::table_row(
                                ui,
                                &[
                                    Cell::code(name.clone(), 0.7),
                                    Cell::number(value.clone(), 0.3),
                                ],
                                false,
                                false,
                                false,
                            );
                        }
                    },
                );
            },
        );
        ui.add_space(space::S2);
        if let Some(t) = &session {
            let minutes = session_minutes(v, &t.session_id);
            widgets::panel(
                ui,
                "睡眠 / 覚醒の推定",
                |ui| {
                    ui.label(muted(ui, sleep_day_label(t)));
                },
                |ui| {
                    if minutes.is_empty() {
                        ui.label(muted(ui, "このセッションの 1 分足はまだありません"));
                        return;
                    }
                    let night = Night::new(&minutes);
                    let group = ui.id().with("dashboard-night");
                    let window = Window::Fixed(night.extent());
                    plot::band(ui, group, &window, &night.band(p));
                    band_legend(ui, p);
                },
            );
        }
    }

    // ------------------------------------------------------------ Sleep

    fn sleep(&mut self, ui: &mut Ui, v: &View) {
        let p = Palette::of(ui);
        let session = self.shown_session(v).cloned();
        // セッションは新しい順に並んでいる。‹ は 1 つ古い夜、› は 1 つ新しい夜。
        let index = session
            .as_ref()
            .and_then(|t| v.sessions.iter().position(|x| x.session_id == t.session_id));
        let older = index
            .and_then(|i| v.sessions.get(i + 1))
            .map(|t| t.session_id.clone());
        let newer = index
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| v.sessions.get(i))
            .map(|t| t.session_id.clone());
        let mut go: Option<String> = None;
        let mut toggle_session = false;
        page_head(ui, "Sleep", |ui| {
            if v.active.is_some() {
                toggle_session = ui.button("セッションを終了").clicked();
            } else {
                ui.label(muted(
                    ui,
                    format!(
                        "睡眠判定が{}分続くと自動記録",
                        self.saved.sleep_confirm_secs / 60
                    ),
                ));
            }
            ui.add_space(space::S2);
            let step = |ui: &mut Ui, label: &str, target: &Option<String>| {
                let button = egui::Button::new(label).frame(false);
                ui.add_enabled(target.is_some(), button).clicked()
            };
            if step(ui, "›", &newer) {
                go = newer.clone();
            }
            if let Some(t) = &session {
                ui.label(sleep_day_label(t));
            }
            if step(ui, "‹", &older) {
                go = older.clone();
            }
        });
        if toggle_session {
            let command = Command::EndSession;
            if self.command(command) {
                self.session = None;
            }
        }
        if go.is_some() {
            self.session = go;
        }

        let Some(t) = session else {
            widgets::panel(
                ui,
                "睡眠セッション",
                |_| {},
                |ui| {
                    widgets::empty_state(
                        ui,
                        "まだセッションがありません",
                        "RR取得中に睡眠判定が設定時間続くと、自動で記録を開始します。",
                        size::PLOT_H_LG,
                    );
                },
            );
            return;
        };
        ui.columns(4, |c| {
            let (h, m) = hours_minutes(t.estimated_sleep_minutes);
            let state = if t.ended_at.is_some() {
                "完了"
            } else if day_is_active(v, &t.session_id) {
                "記録中"
            } else {
                "中断"
            };
            widgets::stat_tile_parts(
                &mut c[0],
                "推定睡眠時間",
                &[(&h, "時間"), (&m, "分")],
                state,
                true,
            );
            let lowest = format!("最低 {} bpm", opt(t.min_hr, 0));
            widgets::stat_tile(
                &mut c[1],
                "平均心拍",
                &opt(t.avg_hr, 0),
                "bpm",
                &lowest,
                false,
            );
            let baseline = match v.baseline.rmssd {
                Some(b) => format!("Baseline {b:.0} ms"),
                None => "Baseline 計測中".to_owned(),
            };
            widgets::stat_tile(
                &mut c[2],
                "RMSSD 中央値",
                &opt(t.median_rmssd, 0),
                "ms",
                &baseline,
                false,
            );
            let times = format!(
                "入眠 {} · 起床 {}",
                time_opt(t.estimated_sleep_at),
                time_opt(t.final_wake_at)
            );
            widgets::stat_tile(
                &mut c[3],
                "覚醒候補",
                &t.awakening_count.to_string(),
                "回",
                &times,
                false,
            );
        });
        ui.add_space(space::S2);

        let minutes = session_minutes(v, &t.session_id);
        widgets::panel(
            ui,
            "睡眠 / 覚醒の推定",
            |ui| {
                let segments = v
                    .days
                    .iter()
                    .find(|d| d.summary.session_id == t.session_id)
                    .map_or(1, |d| d.segments.len());
                ui.label(muted(
                    ui,
                    format!(
                        "{segments} 区間を日単位に集約 · {} 分ぶんの記録（欠測は除外）",
                        minutes.len()
                    ),
                ));
            },
            |ui| {
                if minutes.is_empty() {
                    widgets::empty_state(
                        ui,
                        "1 分足がまだありません",
                        "記録を始めて 1 分たつと、ここにグラフが出ます。",
                        size::PLOT_H_LG,
                    );
                    return;
                }
                let night = Night::new(&minutes);
                // セッションごとに別のグラフとして扱う（前の夜のズームを持ち越さない）
                ui.push_id(&t.session_id, |ui| {
                    let group = ui.id().with("night");
                    let free = Window::Free;
                    plot::band(ui, group, &free, &night.band(p));
                    band_legend(ui, p);

                    let prob = night.runs(|m| m.sleep_probability);
                    let prob_gaps = gaps(&prob, night.extent());
                    ui.add_space(space::S2);
                    widgets::chart_head(ui, "Sleep Probability", "", read(ui, group, &prob, plot::CLOCK, 2, ""));
                    plot::row(
                        ui,
                        group,
                        &free,
                        plot::CLOCK,
                        &Row {
                            ticks: Some(&[0.0, AWAKE_BELOW, ASLEEP_ABOVE, 1.0]),
                            thresholds: &[AWAKE_BELOW, ASLEEP_ABOVE],
                            decimals: 2,
                            wash: true,
                            gaps: &prob_gaps,
                            ..line("prob", "Sleep Probability", p.series_sleep, &prob, YAxis::Fixed(0.0, 1.0))
                        },
                    );

                    let hr = night.runs(|m| m.features.hr_mean);
                    let hr_gaps = gaps(&hr, night.extent());
                    ui.add_space(space::S2);
                    widgets::chart_head(ui, "心拍数", "bpm", read(ui, group, &hr, plot::CLOCK, 0, "bpm"));
                    plot::row(
                        ui,
                        group,
                        &free,
                        plot::CLOCK,
                        &Row {
                            gaps: &hr_gaps,
                            reference: v.baseline.resting_hr,
                            ..line("hr", "心拍数", p.series_hr, &hr, YAxis::Fit { min_span: 10.0, floor: None })
                        },
                    );

                    let rmssd = night.runs(|m| m.features.rmssd);
                    let rmssd_gaps = gaps(&rmssd, night.extent());
                    ui.add_space(space::S2);
                    widgets::chart_head(ui, "HRV", "RMSSD · ms", read(ui, group, &rmssd, plot::CLOCK, 0, "ms"));
                    plot::row(
                        ui,
                        group,
                        &free,
                        plot::CLOCK,
                        &Row {
                            gaps: &rmssd_gaps,
                            reference: v.baseline.rmssd,
                            ..line("rmssd", "RMSSD", p.series_hrv, &rmssd, YAxis::Fit { min_span: 20.0, floor: Some(0.0) })
                        },
                    );

                    let quality = night.runs(|m| Some(m.features.quality * 100.0));
                    let quality_gaps = gaps(&quality, night.extent());
                    ui.add_space(space::S2);
                    widgets::chart_head(ui, "信号品質", "%", read(ui, group, &quality, plot::CLOCK, 0, "%"));
                    plot::row(
                        ui,
                        group,
                        &free,
                        plot::CLOCK,
                        &Row {
                            ticks: Some(&[40.0, QUALITY_FLOOR, 100.0]),
                            thresholds: &[QUALITY_FLOOR],
                            gaps: &quality_gaps,
                            height: size::PLOT_H_SM,
                            show_x_axis: true,
                            ..line("quality", "信号品質", p.series_neutral, &quality, YAxis::Fixed(40.0, 100.0))
                        },
                    );
                    tag_legend(ui, p, false, v.baseline.resting_hr.is_some() || v.baseline.rmssd.is_some());
                    ui.label(
                        RichText::new("横にドラッグで移動 · Ctrl + ホイールで拡大 · ダブルクリックで戻る。縦軸は見えている範囲に自動で合います")
                            .text_style(TextStyle::Small)
                            .color(p.ink_faint),
                    );
                });
            },
        );
        ui.add_space(space::S2);
        widgets::notice(
            ui,
            Status::Off,
            "SleepECG公式GRUの推定は約4〜5分遅延します。HMD静止・装着条件も使いますが、VR利用中の精度は未検証です。",
        );
    }

    // ------------------------------------------------------------ Realtime

    fn realtime(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        if !s.rr_available {
            self.realtime_hr_only(ui, s, v);
            return;
        }
        let p = Palette::of(ui);
        let mut picked = self.window_secs;
        page_head(ui, "Realtime", |ui| {
            window_picker(ui, &mut picked);
            ui.add_space(space::S2);
            tracking_badge(ui, s);
            let quality = format!("信号品質 {:.0}%", s.signal_quality * 100.0);
            let status = if !s.sensor_connected {
                Status::Off
            } else if s.signal_quality * 100.0 < QUALITY_FLOOR {
                Status::Warn
            } else {
                Status::Ok
            };
            widgets::badge(ui, status, &quality);
        });
        self.window_secs = picked;
        let window = self.window_secs;
        ui.columns(4, |c| {
            let f = &s.features;
            let delta = format!("Baseline 比 {} bpm", signed(f.baseline_delta, 0));
            widgets::stat_tile(
                &mut c[0],
                "心拍数",
                &opt(s.heart_rate, 0),
                "bpm",
                &delta,
                true,
            );
            let valid = format!("有効 RR {} 個 / 直近 1 分", f.valid_rr);
            widgets::stat_tile(
                &mut c[1],
                "RR Interval",
                &opt(s.last_rr_ms, 0),
                "ms",
                &valid,
                false,
            );
            let sdnn = format!("SDNN {} ms", opt(f.sdnn, 0));
            widgets::stat_tile(
                &mut c[2],
                "RMSSD",
                &opt(s.rmssd_fast, 0),
                "ms",
                &sdnn,
                false,
            );
            let slope = format!("心拍の傾き {} bpm/分", signed(f.hr_slope, 1));
            widgets::stat_tile(&mut c[3], "pNN50", &opt(f.pnn50, 0), "%", &slope, false);
        });
        ui.add_space(space::S2);

        let hr = realtime_runs(v, window, |s| connected(s, s.heart_rate.map(f64::from)));
        let rr = realtime_runs(v, window, |s| connected(s, s.last_rr_ms));
        let rmssd = realtime_runs(v, window, |s| connected(s, s.rmssd_fast));
        let prob = realtime_runs(v, window, |s| {
            s.sleep.tracking.then_some(s.sleep.probability)
        });
        let has_baseline = v.baseline.resting_hr.is_some();
        widgets::panel(
            ui,
            "信号",
            |ui| {
                let sequence = format!("1 秒ごとに更新 · シーケンス {}", grouped(s.sequence_id));
                ui.label(muted(ui, sequence));
            },
            |ui| {
                if hr.is_empty() {
                    disconnected_state(ui, size::PLOT_H_LG);
                    return;
                }
                let group = ui.id().with("realtime");
                let fixed = Window::Fixed(-window..=0.0);
                let range = -window..=0.0;
                widgets::chart_head(
                    ui,
                    "心拍数",
                    "bpm",
                    read_or_range(ui, group, &hr, &range, 0, "bpm"),
                );
                plot::row(
                    ui,
                    group,
                    &fixed,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        reference: v.baseline.resting_hr,
                        ..line(
                            "hr",
                            "心拍数",
                            p.series_hr,
                            &hr,
                            YAxis::Fit {
                                min_span: 10.0,
                                floor: None,
                            },
                        )
                    },
                );
                ui.add_space(space::S2);
                widgets::chart_head(
                    ui,
                    "RR Interval",
                    "ms",
                    read_or_range(ui, group, &rr, &range, 0, "ms"),
                );
                plot::row(
                    ui,
                    group,
                    &fixed,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        ..line(
                            "rr",
                            "RR Interval",
                            p.series_neutral,
                            &rr,
                            YAxis::Fit {
                                min_span: 100.0,
                                floor: None,
                            },
                        )
                    },
                );
                ui.add_space(space::S2);
                widgets::chart_head(
                    ui,
                    "HRV",
                    "RMSSD · ms",
                    read_or_range(ui, group, &rmssd, &range, 0, "ms"),
                );
                plot::row(
                    ui,
                    group,
                    &fixed,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        reference: v.baseline.rmssd,
                        ..line(
                            "rmssd",
                            "RMSSD",
                            p.series_hrv,
                            &rmssd,
                            YAxis::Fit {
                                min_span: 20.0,
                                floor: Some(0.0),
                            },
                        )
                    },
                );
                ui.add_space(space::S2);
                let fixed_note = Some(("縦軸は 0〜1 で固定".to_owned(), String::new()));
                widgets::chart_head(
                    ui,
                    "Sleep Probability",
                    "",
                    read(ui, group, &prob, plot::AGO, 2, "").or(fixed_note),
                );
                plot::row(
                    ui,
                    group,
                    &fixed,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        ticks: Some(&[0.0, AWAKE_BELOW, ASLEEP_ABOVE, 1.0]),
                        thresholds: &[AWAKE_BELOW, ASLEEP_ABOVE],
                        decimals: 2,
                        wash: true,
                        show_x_axis: true,
                        ..line(
                            "prob",
                            "Sleep Probability",
                            p.series_sleep,
                            &prob,
                            YAxis::Fixed(0.0, 1.0),
                        )
                    },
                );
                tag_legend(ui, p, true, has_baseline);
            },
        );
    }

    fn realtime_hr_only(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        let p = Palette::of(ui);
        let mut picked = self.window_secs;
        page_head(ui, "Realtime", |ui| {
            window_picker(ui, &mut picked);
            tracking_badge(ui, s);
        });
        self.window_secs = picked;
        widgets::notice(
            ui,
            Status::Warn,
            "HRのみの擬似睡眠です。RR・HRVは生成せず、睡眠計測・保存は行いません。",
        );
        ui.columns(3, |c| {
            widgets::stat_tile(
                &mut c[0],
                "心拍数",
                &opt(s.heart_rate, 0),
                "bpm",
                "受信した値",
                true,
            );
            probability_tile(&mut c[1], s);
            widgets::stat_tile(
                &mut c[2],
                "擬似睡眠状態",
                if !s.sleep.tracking {
                    "判定不可"
                } else if s.sleep.sleep_like {
                    "睡眠寄り"
                } else {
                    "覚醒寄り"
                },
                "",
                "OSC / API / IPCへ出力",
                false,
            );
        });
        let window = self.window_secs;
        let hr = realtime_runs(v, window, |s| connected(s, s.heart_rate.map(f64::from)));
        let probability = realtime_runs(v, window, |s| {
            (s.mode == MeasurementMode::HrOnly && s.sleep.tracking).then_some(s.sleep.probability)
        });
        widgets::panel(
            ui,
            "HR / 擬似睡眠",
            |ui| {
                ui.label(muted(ui, "新着HRは即時反映 · OSC 250 ms"));
            },
            |ui| {
                if hr.is_empty() {
                    disconnected_state(ui, size::PLOT_H_LG);
                    return;
                }
                let group = ui.id().with("hr-only");
                let window = Window::Fixed(-window..=0.0);
                widgets::chart_head(ui, "心拍数", "bpm", None);
                plot::row(
                    ui,
                    group,
                    &window,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        ..line(
                            "hr-only",
                            "心拍数",
                            p.series_hr,
                            &hr,
                            YAxis::Fit {
                                min_span: 10.0,
                                floor: None,
                            },
                        )
                    },
                );
                widgets::chart_head(ui, "擬似睡眠スコア", "0〜1", None);
                plot::row(
                    ui,
                    group,
                    &window,
                    plot::AGO,
                    &Row {
                        now_tag: true,
                        show_x_axis: true,
                        thresholds: &[AWAKE_BELOW, ASLEEP_ABOVE],
                        ..line(
                            "pseudo-sleep",
                            "擬似睡眠",
                            p.series_sleep,
                            &probability,
                            YAxis::Fixed(0.0, 1.0),
                        )
                    },
                );
            },
        );
    }

    // ------------------------------------------------------------ History

    fn history(&mut self, ui: &mut Ui, v: &View) {
        let p = Palette::of(ui);
        let mut days = self.range_days;
        page_head(ui, "History", |ui| {
            widgets::segmented(
                ui,
                &mut days,
                &[(7, "7日"), (30, "30日"), (90, "90日"), (0, "全期間")],
                false,
            );
        });
        self.range_days = days;
        let since = if days == 0 {
            i64::MIN
        } else {
            now_ms() - i64::from(days) * 86_400_000
        };
        let sessions: Vec<&SleepSession> = v
            .sessions
            .iter()
            .filter(|t| t.started_at >= since)
            .collect();
        if sessions.is_empty() {
            widgets::panel(
                ui,
                "履歴",
                |_| {},
                |ui| {
                    widgets::empty_state(
                        ui,
                        "この期間の記録はありません",
                        "期間を広げるか、RR対応センサーで自動睡眠記録を行ってください。",
                        size::PLOT_H_LG,
                    );
                },
            );
            return;
        }

        let mean = |values: Vec<f64>| {
            (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
        };
        let ended: Vec<&&SleepSession> = sessions.iter().filter(|t| t.ended_at.is_some()).collect();
        let average_sleep = mean(ended.iter().map(|t| t.estimated_sleep_minutes).collect());
        ui.columns(4, |c| {
            let nights = format!("記録 {} 夜", sessions.len());
            match average_sleep {
                Some(minutes) => {
                    let (h, m) = hours_minutes(minutes);
                    widgets::stat_tile_parts(
                        &mut c[0],
                        "平均の推定睡眠",
                        &[(&h, "時間"), (&m, "分")],
                        &nights,
                        true,
                    );
                }
                None => widgets::stat_tile(&mut c[0], "平均の推定睡眠", "—", "", &nights, true),
            }
            let onset = mean(
                sessions
                    .iter()
                    .filter_map(|t| t.estimated_sleep_at)
                    .map(minutes_from_noon)
                    .collect(),
            );
            let wake = mean(
                sessions
                    .iter()
                    .filter_map(|t| t.final_wake_at)
                    .map(minutes_from_noon)
                    .collect(),
            );
            let wake_note = format!(
                "起床 {}",
                wake.map_or("—".to_owned(), |m| plot::clock(m + 720.0))
            );
            let onset_text = onset.map_or("—".to_owned(), |m| plot::clock(m + 720.0));
            widgets::stat_tile(
                &mut c[1],
                "平均の入眠時刻",
                &onset_text,
                "",
                &wake_note,
                false,
            );
            let hr = mean(sessions.iter().filter_map(|t| t.avg_hr).collect());
            let lowest = sessions.iter().filter_map(|t| t.min_hr).min();
            let lowest_note = format!("最低 {} bpm", opt(lowest, 0));
            widgets::stat_tile(
                &mut c[2],
                "夜間の平均心拍",
                &opt(hr, 0),
                "bpm",
                &lowest_note,
                false,
            );
            let mut medians: Vec<f64> = sessions.iter().filter_map(|t| t.median_rmssd).collect();
            medians.sort_by(f64::total_cmp);
            let median = medians.get(medians.len() / 2).copied();
            let baseline = match v.baseline.rmssd {
                Some(b) => format!("Baseline {b:.0} ms"),
                None => "Baseline 計測中".to_owned(),
            };
            widgets::stat_tile(
                &mut c[3],
                "RMSSD 中央値",
                &opt(median, 0),
                "ms",
                &baseline,
                false,
            );
        });
        ui.add_space(space::S2);

        // 横軸は「今日を 0 とした日数」。古い順に並べ、記録の無い日で線を切る。
        let mut by_day: Vec<(f64, &SleepSession)> = sessions
            .iter()
            .map(|t| (days_ago(sleep_day_timestamp(t)), *t))
            .collect();
        by_day.sort_by(|a, b| a.0.total_cmp(&b.0));
        let series = |value: fn(&SleepSession) -> Option<f64>| {
            runs(by_day.iter().map(|(d, t)| (*d, value(t))), 1.5)
        };
        let sleep = series(|t| Some(t.estimated_sleep_minutes / 60.0));
        let hr = series(|t| t.avg_hr);
        let rmssd = series(|t| t.median_rmssd);
        let first = by_day.first().map_or(0.0, |d| d.0);
        let start = if days == 0 {
            first - 0.5
        } else {
            0.5 - f64::from(days)
        };
        let window = Window::Fixed(start.min(-0.5)..=0.5);
        widgets::panel(
            ui,
            "日ごとの推移",
            |ui| {
                ui.label(muted(
                    ui,
                    format!(
                        "{}時を区切りに1日1件へ集約。欠測を睡眠時間に加えません",
                        self.saved.sleep_day_start_hour
                    ),
                ));
            },
            |ui| {
                let group = ui.id().with("history");
                widgets::chart_head(
                    ui,
                    "推定睡眠時間",
                    "時間",
                    read(ui, group, &sleep, plot::DAYS, 1, "h"),
                );
                plot::row(
                    ui,
                    group,
                    &window,
                    plot::DAYS,
                    &Row {
                        bars: true,
                        decimals: 0,
                        height: size::PLOT_H_LG,
                        ..line(
                            "sleep",
                            "推定睡眠時間",
                            p.series_sleep,
                            &sleep,
                            YAxis::FromZero,
                        )
                    },
                );
                ui.add_space(space::S2);
                widgets::chart_head(
                    ui,
                    "夜間の平均心拍",
                    "bpm",
                    read(ui, group, &hr, plot::DAYS, 0, "bpm"),
                );
                plot::row(
                    ui,
                    group,
                    &window,
                    plot::DAYS,
                    &Row {
                        reference: v.baseline.resting_hr,
                        ..line(
                            "hr",
                            "夜間の平均心拍",
                            p.series_hr,
                            &hr,
                            YAxis::Fit {
                                min_span: 6.0,
                                floor: None,
                            },
                        )
                    },
                );
                ui.add_space(space::S2);
                widgets::chart_head(
                    ui,
                    "RMSSD 中央値",
                    "ms",
                    read(ui, group, &rmssd, plot::DAYS, 0, "ms"),
                );
                plot::row(
                    ui,
                    group,
                    &window,
                    plot::DAYS,
                    &Row {
                        reference: v.baseline.rmssd,
                        show_x_axis: true,
                        ..line(
                            "rmssd",
                            "RMSSD 中央値",
                            p.series_hrv,
                            &rmssd,
                            YAxis::Fit {
                                min_span: 12.0,
                                floor: Some(0.0),
                            },
                        )
                    },
                );
            },
        );
        ui.add_space(space::S2);

        let shown = self.shown_session(v).map(|t| t.session_id.clone());
        let mut open: Option<String> = None;
        widgets::panel(
            ui,
            "日単位の睡眠",
            |ui| {
                ui.label(muted(ui, "行を選ぶと、その夜を Sleep で開きます"));
            },
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let w = [0.17, 0.09, 0.09, 0.09, 0.12, 0.1, 0.08, 0.09, 0.09, 0.08];
                let titles = [
                    "夜",
                    "開始",
                    "入眠",
                    "起床",
                    "推定睡眠",
                    "平均心拍",
                    "最低",
                    "RMSSD",
                    "覚醒候補",
                    "状態",
                ];
                let header: Vec<Cell> = titles
                    .iter()
                    .zip(w)
                    .enumerate()
                    .map(|(i, (t, w))| {
                        if (1..9).contains(&i) {
                            Cell::number(*t, w)
                        } else {
                            Cell::text(*t, w)
                        }
                    })
                    .collect();
                widgets::table_row(ui, &header, true, false, false);
                for t in &sessions {
                    let (h, m) = hours_minutes(t.estimated_sleep_minutes);
                    let recording = day_is_active(v, &t.session_id);
                    let state = if t.ended_at.is_some() {
                        Cell::text("完了", w[9]).muted()
                    } else if recording {
                        Cell::text("記録中", w[9])
                    } else {
                        Cell::text("中断", w[9]).warn()
                    };
                    let cells = [
                        Cell::text(sleep_day_label(t), w[0]),
                        Cell::number(time(t.started_at), w[1]),
                        Cell::number(time_opt(t.estimated_sleep_at), w[2]),
                        Cell::number(time_opt(t.final_wake_at), w[3]),
                        Cell::number(format!("{h}h {m:0>2}m"), w[4]),
                        Cell::number(opt(t.avg_hr, 0), w[5]),
                        Cell::number(opt(t.min_hr, 0), w[6]),
                        Cell::number(opt(t.median_rmssd, 0), w[7]),
                        Cell::number(t.awakening_count.to_string(), w[8]),
                        state,
                    ];
                    let selected = shown.as_ref() == Some(&t.session_id);
                    if widgets::table_row(ui, &cells, false, selected, true).clicked() {
                        open = Some(t.session_id.clone());
                    }
                }
            },
        );
        if open.is_some() {
            self.session = open;
            self.page = Page::Sleep;
        }
    }

    // ------------------------------------------------------------ VRChat OSC

    fn osc(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        let (status, text) = osc_state(&v.osc_status);
        page_head(ui, "VRChat OSC", |ui| {
            widgets::badge(ui, status, text);
        });
        let saved = self.saved.clone();
        split(
            ui,
            0.46,
            |ui| {
                widgets::panel(
                    ui,
                    "出力設定",
                    |_| {},
                    |ui| {
                        widgets::setting_row(
                            ui,
                            "OSC 出力",
                            "有効にすると、下の宛先へ UDP で送り始めます",
                            |ui| {
                                widgets::toggle(ui, &mut self.settings.osc_enabled);
                            },
                        );
                        widgets::setting_row(
                            ui,
                            "接続先",
                            "OSCQueryでVRChatを自動検出（OFFで手動IP / Port）",
                            |ui| {
                                widgets::toggle(ui, &mut self.settings.oscquery_enabled);
                            },
                        );
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(muted(ui, "Host（IP アドレス）"));
                                widgets::text_field(ui, &mut self.settings.osc_host, 180.0, true);
                            });
                            ui.vertical(|ui| {
                                ui.label(muted(ui, "Port"));
                                widgets::text_field(ui, &mut self.osc_port, 72.0, true);
                            });
                        });
                        ui.add_space(space::S1);
                        ui.separator();
                        widgets::setting_row(
                            ui,
                            "VRCOSC Heartrate 互換",
                            "既存の心拍対応アバターでそのまま動きます",
                            |ui| {
                                widgets::toggle(ui, &mut self.settings.heartrate_output);
                            },
                        );
                        widgets::setting_row(
                            ui,
                            "Sleep 出力",
                            "Probability / Asleep / Estimated / MeasurementAvailable などを送ります",
                            |ui| {
                                widgets::toggle(ui, &mut self.settings.sleep_output);
                            },
                        );
                        self.save_bar(ui);
                    },
                );
            },
            |ui| {
                let target = if saved.oscquery_enabled {
                    "OSCQuery · Auto · 4 Hz".into()
                } else {
                    format!("UDP → {}:{} · 4 Hz", saved.osc_host, saved.osc_port)
                };
                widgets::panel(
                    ui,
                    "送信状況",
                    |ui| widgets::info(ui, &target, true),
                    |ui| {
                        if status == Status::Danger {
                            widgets::notice(ui, Status::Danger, &v.osc_status);
                        } else {
                            widgets::notice(ui, Status::Off, &v.osc_status);
                        }
                        ui.add_space(space::S1);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let w = [0.6, 0.15, 0.25];
                        let header = [
                            Cell::text("Parameter", w[0]),
                            Cell::text("Type", w[1]),
                            Cell::number("現在値", w[2]),
                        ];
                        widgets::table_row(ui, &header, true, false, false);
                        for (name, kind, value) in osc_rows(s, &saved) {
                            let cells = [
                                Cell::code(format!("VRCOSC/{name}"), w[0]),
                                Cell::text(kind, w[1]).muted(),
                                Cell::number(value, w[2]),
                            ];
                            widgets::table_row(ui, &cells, false, false, false);
                        }
                    },
                );
            },
        );
    }

    // ------------------------------------------------------------ Sensors

    fn sensors(&mut self, ui: &mut Ui, s: &PhysioSnapshot, v: &View) {
        let p = Palette::of(ui);
        let mut scan = false;
        let mut disconnect = false;
        let mut connect: Option<String> = None;
        page_head(ui, "Sensors", |ui| {
            scan = ui
                .add_enabled(
                    !s.sensor_connected && !v.connecting,
                    egui::Button::new("デバイスをスキャン"),
                )
                .clicked();
        });
        widgets::panel(
            ui,
            "HypeRate · HRのみ",
            |ui| {
                widgets::badge(ui, Status::Off, "WebSocket");
            },
            |ui| {
                ui.label(muted(
                    ui,
                    "HypeRate IDとAPIキーで接続します。APIキーは保存・外部公開しません。",
                ));
                ui.label(muted(ui, "HypeRate ID"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.hyperate_id)
                        .hint_text("例: abc123")
                        .desired_width(ui.available_width()),
                );
                ui.label(muted(ui, "APIキー"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.hyperate_key)
                        .password(true)
                        .hint_text("HypeRate API key")
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !s.sensor_connected
                                && !v.connecting
                                && !self.demo
                                && !self.hyperate_key.trim().is_empty()
                                && !self.settings.hyperate_id.trim().is_empty(),
                            egui::Button::new("HypeRateに接続"),
                        )
                        .clicked()
                    {
                        let key = std::mem::take(&mut self.hyperate_key);
                        let id = self.settings.hyperate_id.clone();
                        self.command(Command::ConnectHypeRate { id, key });
                    }
                    if ui
                        .add_enabled(
                            s.sensor_connected || v.connecting,
                            egui::Button::new("入力を切断 / 接続を中止"),
                        )
                        .clicked()
                    {
                        self.command(Command::Disconnect);
                    }
                    ui.hyperlink_to("APIキーの公式申請", "https://www.hyperate.io/api.html");
                });
                self.save_bar(ui);
            },
        );
        ui.add_space(space::S2);
        split(
            ui,
            0.54,
            |ui| {
                widgets::panel(
                    ui,
                    "接続中のセンサー",
                    |ui| {
                        if s.sensor_connected {
                            widgets::badge(ui, Status::Ok, "接続中");
                        } else {
                            widgets::badge(ui, Status::Off, "未接続");
                        }
                    },
                    |ui| {
                        if !s.sensor_connected {
                            widgets::empty_state(
                                ui,
                                "センサーがつながっていません",
                                "スキャンして、心拍センサーを選んでください。記録済みのデータは残っています。",
                                size::PLOT_H_LG,
                            );
                            return;
                        }
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(
                                    RichText::new(sensor_name(s, v))
                                        .text_style(text_style("body-strong")),
                                );
                                let id = s.sensor_id.clone().unwrap_or_default();
                                ui.label(
                                    RichText::new(id)
                                        .text_style(TextStyle::Monospace)
                                        .color(p.ink_faint),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let label = RichText::new("切断").color(p.danger);
                                disconnect = ui.button(label).clicked();
                            });
                        });
                        ui.add_space(space::S2);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = space::S8;
                            widgets::key_value(ui, "心拍数", &opt(s.heart_rate, 0), "bpm");
                            widgets::key_value(
                                ui,
                                "信号品質",
                                &format!("{:.0}", s.signal_quality * 100.0),
                                "%",
                            );
                            widgets::key_value(
                                ui,
                                "有効 RR / 直近 1 分",
                                &s.features.valid_rr.to_string(),
                                "個",
                            );
                            widgets::key_value(ui, "バッテリー", "—", "");
                        });
                        ui.add_space(space::S4);
                        let quality = realtime_runs(v, 1800.0, |s| {
                            connected(s, Some(s.signal_quality * 100.0))
                        });
                        widgets::chart_head(ui, "信号品質", "直近 30 分 · %", None);
                        let group = ui.id().with("quality");
                        plot::row(
                            ui,
                            group,
                            &Window::Fixed(-1800.0..=0.0),
                            plot::AGO,
                            &Row {
                                ticks: Some(&[40.0, QUALITY_FLOOR, 100.0]),
                                thresholds: &[QUALITY_FLOOR],
                                now_tag: true,
                                show_x_axis: true,
                                ..line(
                                    "quality",
                                    "信号品質",
                                    p.series_neutral,
                                    &quality,
                                    YAxis::Fixed(40.0, 100.0),
                                )
                            },
                        );
                        let note = format!("{QUALITY_FLOOR:.0}% を下回ると Tracking を解除します");
                        ui.label(
                            RichText::new(note)
                                .text_style(TextStyle::Small)
                                .color(p.ink_faint),
                        );
                    },
                );
            },
            |ui| {
                widgets::panel(
                    ui,
                    "見つかったデバイス",
                    |ui| {
                        ui.label(muted(ui, format!("{} 台", v.devices.len())));
                    },
                    |ui| {
                        if v.devices.is_empty() {
                            ui.label(muted(ui, "まだスキャンしていません。「デバイスをスキャン」を押してください。"));
                        }
                        for d in &v.devices {
                            let current = s.sensor_connected && s.sensor_id.as_ref() == Some(&d.id);
                            ui.horizontal(|ui| {
                                ui.set_min_height(size::ROW_H + space::S2);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    ui.label(
                                        RichText::new(&d.name)
                                            .text_style(text_style("body-strong")),
                                    );
                                    ui.label(
                                        RichText::new(&d.id)
                                            .text_style(TextStyle::Monospace)
                                            .color(p.ink_faint),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if current {
                                        widgets::badge(ui, Status::Ok, "接続中");
                                    } else if ui
                                        .add_enabled(
                                            !s.sensor_connected && !v.connecting,
                                            egui::Button::new("接続"),
                                        )
                                        .clicked()
                                    {
                                        connect = Some(d.id.clone());
                                    }
                                });
                            });
                            ui.separator();
                        }
                        if s.sensor_connected && !v.devices.is_empty() {
                            ui.label(muted(ui, "別のセンサーへつなぐには、先に切断してください"));
                        }
                    },
                );
                ui.add_space(space::S2);
                widgets::panel(
                    ui,
                    "接続の動作",
                    |_| {},
                    |ui| {
                        widgets::setting_row(
                            ui,
                            "自動再接続",
                            "切断されたら同じセンサーへつなぎ直します",
                            |ui| {
                                widgets::info(ui, "3 秒後に再試行", false);
                            },
                        );
                        widgets::setting_row(
                            ui,
                            "無受信の扱い",
                            "通知が途切れたら Tracking を解除します",
                            |ui| {
                                widgets::info(ui, "5 秒", false);
                            },
                        );
                        widgets::setting_row(
                            ui,
                            "対応センサー",
                            "Coospo、Polar など",
                            |ui| {
                                widgets::info(ui, "標準 Heart Rate Service", false);
                            },
                        );
                    },
                );
            },
        );
        if scan {
            self.command(Command::Scan);
        }
        if disconnect {
            self.command(Command::Disconnect);
        }
        if let Some(id) = connect {
            self.command(Command::Connect(id));
        }
    }

    // ------------------------------------------------------------ Settings

    fn settings_page(&mut self, ui: &mut Ui, v: &View) {
        page_head(ui, "Settings", |_| {});
        ui.columns(2, |c| {
            widgets::panel(
                &mut c[0],
                "睡眠推定",
                |_| {},
                |ui| {
                    widgets::notice(
                        ui,
                        Status::Warn,
                        "RR睡眠はSleepECG公式GRU（約4〜5分遅延）。VR環境での精度は未検証です。HRのみの擬似判定は別扱いです。",
                    );
                    widgets::setting_row(ui, "睡眠推定を有効にする", "RRから公式GRUで推定。モデル未準備時は判定不能になります", |ui| {
                        widgets::toggle(ui, &mut self.settings.sleep_enabled);
                    });
                    ui.label(muted(ui,"睡眠モデル用Python実行ファイル（専用venvのpython / python.exe）"));
                    ui.add(egui::TextEdit::singleline(&mut self.settings.sleep_python).desired_width(f32::INFINITY));
                    let current=self.core.latest();
                    ui.label(muted(ui,&current.sleep_model.message));
                    widgets::setting_row(ui,"HMDの静止・装着条件","OpenVR取得不能／未装着／加速度平均0では棄却。OFFでRRモデル単独",|ui|{widgets::toggle(ui,&mut self.settings.sleep_hmd_enabled);});
                    ui.label(muted(ui,format!("{} · 加速度平均 {} m/s² · 装着 {}",current.hmd.message,opt(current.hmd.acceleration_mean,4),match current.hmd.worn{Some(true)=>"あり",Some(false)=>"なし",None=>"取得不能"})));
                    widgets::setting_row(ui,"睡眠日の区切り","夜間の分割を避けるため初期値は正午。既存の日付は維持します",|ui|{ui.add(egui::DragValue::new(&mut self.settings.sleep_day_start_hour).range(0..=23).suffix(" 時"));});
                    widgets::setting_row(ui,"自動睡眠記録","RRありの睡眠判定が続いたら記録を開始します",|ui|{widgets::toggle(ui,&mut self.settings.auto_sleep_recording);});
                    widgets::setting_row(ui,"睡眠確定までの時間","擬似睡眠 / RR睡眠の共通条件。60〜1800秒",|ui|{ui.add(egui::DragValue::new(&mut self.settings.sleep_confirm_secs).range(60..=1800).suffix(" 秒"));});
                    widgets::setting_row(ui,"擬似睡眠の基準心拍","HRのみの暫定ルール。初期値60 bpm",|ui|{ui.add(egui::DragValue::new(&mut self.settings.pseudo_sleep_hr).range(35..=100).suffix(" bpm"));});
                    widgets::setting_row(ui, "Baseline の期間", "個人内の変化を比べる基準になる日数です", |ui| {
                        widgets::segmented(ui, &mut self.settings.baseline_days, &[(14, "14日"), (30, "30日")], false);
                    });
                    widgets::setting_row(ui, "Baseline の状態", "グラフの個人平均との差。公式GRUの開始条件とは別です", |ui| {
                        ui.label(muted(ui, "分"));
                        ui.label(RichText::new(grouped(v.baseline.sample_minutes as u64)).text_style(text_style("value")));
                    });
                },
            );
            c[0].add_space(space::S2);
            widgets::panel(
                &mut c[0],
                "表示",
                |_| {},
                |ui| {
                    widgets::setting_row(ui, "テーマ", "いまは保存されません。次の起動ではダークに戻ります", |ui| {
                        use egui::ThemePreference::{Dark, Light, System};
                        let options = [(Dark, "ダーク"), (Light, "ライト"), (System, "システム")];
                        if widgets::segmented(ui, &mut self.theme, &options, false) {
                            ui.ctx().set_theme(self.theme);
                        }
                    });
                },
            );
            widgets::panel(
                &mut c[1],
                "外部連携",
                |_| {},
                |ui| {
                    widgets::setting_row(ui, "ローカル REST API", "localhost限定。/api/v1/sleep/status で睡眠状態・擬似判定の別も取得できます", |ui| {
                        widgets::toggle(ui, &mut self.settings.api_enabled);
                    });
                    widgets::setting_row(ui, "API ポート", "1〜65535", |ui| {
                        widgets::text_field(ui, &mut self.api_port, 72.0, true);
                    });
                    widgets::setting_row(ui,"Web心拍表示","XSOverlay / OVR Toolkitのブラウザ用。REST APIとは別に有効化できます",|ui|{widgets::toggle(ui,&mut self.settings.overlay_enabled);});
                    let overlay_url=format!("http://127.0.0.1:{}/overlay/heart-rate",self.saved.api_port);
                    ui.hyperlink_to("心拍Webページを開く",&overlay_url);
                    ui.label(muted(ui,&overlay_url));
                    widgets::setting_row(ui,"閉じてもトレイに常駐","計測とWeb配信を継続。トレイメニューの「終了」で保存して停止します",|ui|{widgets::toggle(ui,&mut self.settings.close_to_tray);});
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.resident.available(),egui::Button::new("トレイに隠す")).clicked(){self.resident.hide(ui.ctx());}
                        if ui.button("NemuriBeatを終了").clicked(){self.resident.quit(ui.ctx());}
                    });
                    widgets::setting_row(ui, "Unix ソケット", "起動時に --ipc で指定します。Windows は準備中です", |ui| {
                        widgets::info(ui, "起動時に指定", false);
                    });
                },
            );
            c[1].add_space(space::S2);
            widgets::panel(&mut c[1], "OyasumiVR", |_| {}, |ui| {
                widgets::setting_row(ui, "睡眠モードを連携", "NemuriBeatの判定でOyasumiVRの睡眠モードをON/OFFします", |ui| {
                    widgets::toggle(ui, &mut self.settings.oyasumi_enabled);
                });
                widgets::setting_row(ui, "HRのみの擬似判定も送る", "HypeRate等の暫定判定を使用。初期値OFF", |ui| {
                    widgets::toggle(ui, &mut self.settings.oyasumi_allow_hr_only);
                });
                widgets::setting_row(ui, "OSCQueryポート", "0で自動検出。手動の場合はOyasumiVRのOSCQuery受信ポート", |ui| {
                    ui.add(egui::DragValue::new(&mut self.settings.oyasumi_query_port).range(0..=65535));
                });
                ui.label(muted(ui, &v.oyasumi_status));
                ui.label(muted(ui, "OyasumiVR側のOSCサーバーを有効にしてください。判定不能時は状態を保持します。"));
                ui.label(muted(ui, "OyasumiVRで設定済みの明るさ・マイク等の自動操作が実行されます。独自の睡眠検出との競合に注意してください。"));
            });
            c[1].add_space(space::S2);
            widgets::panel(
                &mut c[1],
                "データ",
                |_| {},
                |ui| {
                    widgets::setting_row(ui, "保存場所", &self.data_dir, |ui| {
                        widgets::info(ui, "--data-dir で変更", false);
                    });
                    widgets::setting_row(ui, "PersonalData", "実測HR/RR・設定・確定睡眠を永続保存します", |ui| {
                        widgets::info(ui, "すべて保持", false);
                    });
                    widgets::setting_row(ui,"Cache","再生成可能な集計のみ。35日を超えた集計を削除",|ui|{widgets::info(ui,"一時データ",false);});
                    widgets::setting_row(ui, "ログ", "出力レベルは環境変数で指定します", |ui| {
                        widgets::info(ui, "RUST_LOG", true);
                    });
                },
            );
        });
        ui.add_space(space::S2);
        let p = Palette::of(ui);
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.save_bar(ui);
        });
    }

    /// 設定の変更をまとめて反映する行。VRChat OSC と Settings で共通。
    fn save_bar(&mut self, ui: &mut Ui) {
        let mut error = None;
        match self.osc_port.trim().parse::<u16>() {
            Ok(port) if port > 0 => self.settings.osc_port = port,
            _ => error = Some("OSC の Port は 1〜65535 で入力してください".to_owned()),
        }
        match self.api_port.trim().parse::<u16>() {
            Ok(port) if port > 0 => self.settings.api_port = port,
            _ => error = Some("API ポートは 1〜65535 で入力してください".to_owned()),
        }
        if error.is_none() {
            error = self.settings.validate().err().map(|e| localize(&e));
        }
        let port_edits = usize::from(self.osc_port.trim() != self.saved.osc_port.to_string())
            + usize::from(self.api_port.trim() != self.saved.api_port.to_string());
        let changes = changes(&self.settings, &self.saved).max(port_edits);
        match widgets::save_bar(ui, changes, error.as_deref()) {
            SaveAction::Apply => {
                if self.command(Command::Save(self.settings.clone())) {
                    self.saved = self.settings.clone();
                }
            }
            SaveAction::Revert => {
                self.settings = self.saved.clone();
                self.osc_port = self.saved.osc_port.to_string();
                self.api_port = self.saved.api_port.to_string();
            }
            SaveAction::None => {}
        }
    }
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let url = format!(
            "http://127.0.0.1:{}/overlay/heart-rate",
            self.saved.api_port
        );
        if let Some(error) = self.resident.update(ctx, self.saved.close_to_tray, url) {
            self.notice = error;
        }
        if self.resident.visible() {
            self.draw(ctx);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

// ---------------------------------------------------------------- 画面の部品

/// 画面のタイトル行。`right` は右端から左へ並ぶ。
fn page_head(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(size::CONTROL_H);
        ui.heading(title);
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(space::S4);
}

/// 左右 2 列。`left` は左の列の幅の割合。
fn split(ui: &mut Ui, left: f32, a: impl FnOnce(&mut Ui), b: impl FnOnce(&mut Ui)) {
    let gap = space::S4;
    let total = ui.available_width() - gap;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let column = Layout::top_down(Align::Min);
        ui.allocate_ui_with_layout(egui::vec2(total * left, 0.0), column, |ui| {
            ui.set_width(total * left);
            a(ui);
        });
        ui.allocate_ui_with_layout(egui::vec2(total * (1.0 - left), 0.0), column, |ui| {
            ui.set_width(total * (1.0 - left));
            b(ui);
        });
    });
}

fn window_picker(ui: &mut Ui, seconds: &mut f64) {
    widgets::segmented(
        ui,
        seconds,
        &[(60.0, "1分"), (300.0, "5分"), (1800.0, "30分")],
        true,
    );
}

fn tracking_badge(ui: &mut Ui, s: &PhysioSnapshot) {
    if s.sleep.tracking {
        widgets::badge(
            ui,
            if s.mode == MeasurementMode::HrOnly {
                Status::Warn
            } else {
                Status::Ok
            },
            if s.mode == MeasurementMode::HrOnly {
                "擬似睡眠 · HRのみ"
            } else {
                "Tracking · HR + RR"
            },
        );
    } else {
        widgets::badge(ui, Status::Warn, "判定不能");
    }
}

fn probability_tile(ui: &mut Ui, s: &PhysioSnapshot) {
    let p = Palette::of(ui);
    theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(widgets::TILE_MIN_H);
        ui.spacing_mut().item_spacing.y = space::S1;
        ui.horizontal(|ui| {
            ui.label(muted(
                ui,
                if s.mode == MeasurementMode::HrOnly {
                    "擬似睡眠スコア"
                } else {
                    "Sleep Probability"
                },
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                tracking_badge(ui, s)
            });
        });
        let value = s.sleep.tracking.then_some(s.sleep.probability);
        let ink = if value.is_some() { p.ink } else { p.ink_faint };
        ui.label(
            RichText::new(opt(value, 2))
                .text_style(text_style("stat"))
                .color(ink),
        );
        widgets::probability_meter(
            ui,
            value.map(|v| v as f32),
            AWAKE_BELOW as f32,
            ASLEEP_ABOVE as f32,
        );
    });
}

fn disconnected_state(ui: &mut Ui, height: f32) {
    widgets::empty_state(
        ui,
        "センサーが切断されています",
        "再接続すると、ここに値が流れます。記録済みのデータは残っています。",
        height,
    );
}

fn band_legend(ui: &mut Ui, p: &Palette) {
    widgets::legend(
        ui,
        &[
            (p.series_sleep, "睡眠（0.75 以上）"),
            (p.sleep_transition, "移行"),
            (p.sleep_awake, "覚醒（0.40 以下）"),
            (p.plot_gap, "未計測（Tracking = false）"),
        ],
    );
}

fn tag_legend(ui: &mut Ui, p: &Palette, now: bool, baseline: bool) {
    let mut items = Vec::new();
    if now {
        items.push((p.ink, "現在値"));
    }
    if baseline {
        items.push((p.surface, "Baseline"));
    }
    items.push((p.surface_raised, "カーソル位置の値"));
    widgets::legend(ui, &items);
}

/// 既定の設定で折れ線 1 段ぶんの `Row` を作る。必要な項目だけ上書きして使う。
fn line<'a>(id: &'static str, name: &'a str, color: Color32, runs: &'a Runs, y: YAxis) -> Row<'a> {
    Row {
        id,
        name,
        color,
        runs,
        y,
        ticks: None,
        decimals: 0,
        height: size::PLOT_H,
        wash: false,
        bars: false,
        now_tag: false,
        reference: None,
        thresholds: &[],
        band: None,
        gaps: &[],
        show_x_axis: false,
    }
}

/// ホバー中なら、その時刻と値。
fn read(
    ui: &Ui,
    group: egui::Id,
    runs: &Runs,
    axis: plot::XAxis,
    decimals: usize,
    unit: &str,
) -> Option<(String, String)> {
    let [t, v] = plot::nearest(runs, plot::hovered_time(ui, group)?)?;
    Some((
        (axis.label)(t),
        format!("{v:.decimals$} {unit}").trim_end().to_owned(),
    ))
}

/// ホバー中はその時刻と値、そうでなければ見えている範囲の最小–最大。
fn read_or_range(
    ui: &Ui,
    group: egui::Id,
    runs: &Runs,
    window: &RangeInclusive<f64>,
    decimals: usize,
    unit: &str,
) -> Option<(String, String)> {
    read(ui, group, runs, plot::AGO, decimals, unit).or_else(|| {
        let values = runs
            .iter()
            .flatten()
            .filter(|p| window.contains(&p[0]))
            .map(|p| p[1]);
        let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
        (lo <= hi).then(|| {
            (
                "表示範囲".to_owned(),
                format!("{lo:.decimals$}–{hi:.decimals$} {unit}"),
            )
        })
    })
}

// ---------------------------------------------------------------- データの整形

/// 値のある点が続く区間ごとに分ける。`None` と、`max_gap` を超える時間の空きで区間を切る。
fn runs(points: impl Iterator<Item = (f64, Option<f64>)>, max_gap: f64) -> Runs {
    let mut out: Runs = Vec::new();
    let mut last: Option<f64> = None;
    for (t, v) in points {
        let Some(v) = v.filter(|v| v.is_finite()) else {
            last = None;
            continue;
        };
        if last.is_none_or(|l| t - l > max_gap) {
            out.push(Vec::new());
        }
        if let Some(run) = out.last_mut() {
            run.push([t, v]);
        }
        last = Some(t);
    }
    out
}

/// 区間と区間の間（未計測）。記録の端から最初・最後の区間までの空きも含める。
fn gaps(runs: &Runs, extent: RangeInclusive<f64>) -> Vec<RangeInclusive<f64>> {
    let mut out = Vec::new();
    let mut cursor = *extent.start();
    for run in runs {
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            continue;
        };
        if first[0] - cursor > 1.5 {
            out.push(cursor..=first[0]);
        }
        cursor = last[0];
    }
    if *extent.end() - cursor > 1.5 {
        out.push(cursor..=*extent.end());
    }
    out
}

/// 直近 `window` 秒のスナップショットから 1 系列を作る。横軸は「現在からの秒」。
fn realtime_runs(v: &View, window: f64, value: impl Fn(&PhysioSnapshot) -> Option<f64>) -> Runs {
    let now = v.display.back().map_or_else(now_ms, |s| s.timestamp);
    let points = v
        .display
        .iter()
        .map(|s| ((s.timestamp - now) as f64 / 1000.0, value(s)))
        .filter(|(t, _)| *t >= -window - 5.0);
    runs(points, 5.0)
}

fn connected(s: &PhysioSnapshot, value: Option<f64>) -> Option<f64> {
    value.filter(|_| s.sensor_connected)
}

/// あるセッションの 1 分足。読み込み中や別のセッションのものは除く。
fn session_minutes<'a>(v: &'a View, id: &str) -> Vec<&'a FeatureMinute> {
    let members = v.days.iter().find(|d| d.summary.session_id == id);
    v.history
        .iter()
        .filter(|m| {
            m.session_id.as_deref() == Some(id)
                || members.is_some_and(|d| {
                    d.segments
                        .iter()
                        .any(|s| Some(s.session_id.as_str()) == m.session_id.as_deref())
                })
        })
        .collect()
}
fn day_is_active(v: &View, id: &str) -> bool {
    v.active.as_ref().is_some_and(|a| {
        a.session_id == id
            || v.days
                .iter()
                .find(|d| d.summary.session_id == id)
                .is_some_and(|d| d.segments.iter().any(|s| s.session_id == a.session_id))
    })
}
fn sleep_day_label(t: &SleepSession) -> String {
    t.sleep_day
        .clone()
        .unwrap_or_else(|| night_label(t.started_at))
}
fn sleep_day_timestamp(t: &SleepSession) -> i64 {
    use chrono::TimeZone;
    t.sleep_day
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .and_then(|d| chrono::Local.from_local_datetime(&d).single())
        .map_or(t.started_at, |d| d.timestamp_millis())
}

/// 一晩ぶんの 1 分足。横軸は「始めた日の 0:00 からの分」。
struct Night<'a> {
    minutes: &'a [&'a FeatureMinute],
    base: i64,
}

impl<'a> Night<'a> {
    fn new(minutes: &'a [&'a FeatureMinute]) -> Self {
        let first = minutes.first().map_or(0, |m| m.timestamp);
        Self {
            minutes,
            base: local_midnight(first),
        }
    }

    fn x(&self, timestamp: i64) -> f64 {
        (timestamp - self.base) as f64 / 60_000.0
    }

    fn extent(&self) -> RangeInclusive<f64> {
        let first = self.minutes.first().map_or(0.0, |m| self.x(m.timestamp));
        let last = self
            .minutes
            .last()
            .map_or(1.0, |m| self.x(m.timestamp) + 1.0);
        first..=last.max(first + 1.0)
    }

    fn runs(&self, value: impl Fn(&FeatureMinute) -> Option<f64>) -> Runs {
        runs(
            self.minutes.iter().map(|m| (self.x(m.timestamp), value(m))),
            1.5,
        )
    }

    /// 睡眠 / 移行 / 覚醒 / 未計測の区間。同じ種類が続く分はまとめる。
    fn band(&self, p: &Palette) -> Vec<(RangeInclusive<f64>, Color32)> {
        let mut out: Vec<(RangeInclusive<f64>, Color32)> = Vec::new();
        let mut push = |from: f64, to: f64, color: Color32| match out.last_mut() {
            Some((range, c)) if *c == color && (from - *range.end()).abs() < 0.05 => {
                *range = *range.start()..=to
            }
            _ => out.push((from..=to, color)),
        };
        let mut previous_end: Option<f64> = None;
        for m in self.minutes {
            let x = self.x(m.timestamp);
            if let Some(end) = previous_end.filter(|end| x - end > 0.5) {
                push(end, x, p.plot_gap);
            }
            let color = match m.sleep_probability {
                Some(v) if v >= ASLEEP_ABOVE => p.series_sleep,
                Some(v) if v > AWAKE_BELOW => p.sleep_transition,
                Some(_) => p.sleep_awake,
                None => p.plot_gap,
            };
            push(x, x + 1.0, color);
            previous_end = Some(x + 1.0);
        }
        out
    }
}

/// OSC で送っているパラメーター（名前、型、いまの値）。
fn osc_rows(s: &PhysioSnapshot, settings: &Settings) -> Vec<(String, &'static str, String)> {
    osc_vrchat::parameters(s, settings, false)
        .into_iter()
        .map(|m| {
            let name = m
                .addr
                .trim_start_matches("/avatar/parameters/VRCOSC/")
                .to_owned();
            let (kind, value) = match m.args.first() {
                Some(rosc::OscType::Bool(b)) => ("Bool", b.to_string()),
                Some(rosc::OscType::Int(i)) => ("Int", i.to_string()),
                Some(rosc::OscType::Float(f)) => ("Float", format!("{f:.3}")),
                _ => ("—", "—".to_owned()),
            };
            (name, kind, value)
        })
        .collect()
}

/// runtime が出す OSC の状態の文を、バッジの種類と語にする。
fn osc_state(status: &str) -> (Status, &'static str) {
    if status.starts_with("OSCQuery: VRChat検出・") {
        (Status::Ok, "検出・送信中")
    } else if status.starts_with("OSCQuery") {
        (Status::Off, "検出待ち")
    } else if status.starts_with("UDP") {
        (Status::Ok, "送信中")
    } else if status.starts_with("OSC error") {
        (Status::Danger, "送信エラー")
    } else {
        (Status::Off, "無効")
    }
}

/// 編集中の設定と、いま動いている設定で違う項目の数。
fn changes(a: &Settings, b: &Settings) -> usize {
    [
        a.oyasumi_enabled != b.oyasumi_enabled,
        a.oyasumi_allow_hr_only != b.oyasumi_allow_hr_only,
        a.oyasumi_query_port != b.oyasumi_query_port,
        a.osc_enabled != b.osc_enabled,
        a.oscquery_enabled != b.oscquery_enabled,
        a.osc_host != b.osc_host,
        a.osc_port != b.osc_port,
        a.heartrate_output != b.heartrate_output,
        a.sleep_output != b.sleep_output,
        a.sleep_enabled != b.sleep_enabled,
        a.baseline_days != b.baseline_days,
        a.api_enabled != b.api_enabled,
        a.overlay_enabled != b.overlay_enabled,
        a.close_to_tray != b.close_to_tray,
        a.hyperate_id != b.hyperate_id,
        a.api_port != b.api_port,
        a.sleep_confirm_secs != b.sleep_confirm_secs,
        a.pseudo_sleep_hr != b.pseudo_sleep_hr,
        a.auto_sleep_recording != b.auto_sleep_recording,
        a.sleep_python != b.sleep_python,
        a.sleep_hmd_enabled != b.sleep_hmd_enabled,
        a.sleep_day_start_hour != b.sleep_day_start_hour,
    ]
    .into_iter()
    .filter(|changed| *changed)
    .count()
}

fn sensor_name(s: &PhysioSnapshot, v: &View) -> String {
    let id = s.sensor_id.clone().unwrap_or_default();
    v.devices
        .iter()
        .find(|d| d.id == id)
        .map_or(id.clone(), |d| d.name.clone())
}

/// runtime の英語のメッセージを、画面の言葉に直す。知らない文はそのまま出す。
fn localize(message: &str) -> String {
    let fixed = [
        (
            "Disconnect before scanning",
            "スキャンする前に、センサーを切断してください",
        ),
        (
            "Demo mode: restart without --demo to connect BLE",
            "デモモードです。センサーにつなぐには --demo なしで起動してください",
        ),
        ("Scanning BLE devices…", "センサーを探しています…"),
        (
            "Scan complete; select a heart rate sensor",
            "スキャンが終わりました。心拍センサーを選んでください",
        ),
        (
            "Settings queue full: not applied",
            "設定を適用できませんでした。もう一度お試しください",
        ),
        (
            "Settings applied; persistence is asynchronous",
            "設定を適用しました",
        ),
        (
            "Sensor connected; notifications active",
            "センサーに接続しました。通知を受信しています",
        ),
        (
            "OSC host must be an IP address",
            "Host には IP アドレスを入力してください",
        ),
        (
            "Ports must be nonzero",
            "ポートは 1〜65535 で入力してください",
        ),
        (
            "Baseline period must be 14 or 30 days",
            "Baseline の期間は 14 日か 30 日です",
        ),
    ];
    if let Some((_, ja)) = fixed.iter().find(|(en, _)| *en == message) {
        return (*ja).to_owned();
    }
    match message.strip_suffix("; automatic retry in 3 s") {
        Some(reason) => format!("{reason}。3 秒後に再接続します"),
        None => message.to_owned(),
    }
}

// ---------------------------------------------------------------- 書式

/// 値が無いときは `—`。
fn opt<T: Into<f64>>(value: Option<T>, decimals: usize) -> String {
    value.map_or("—".to_owned(), |v| format!("{:.decimals$}", v.into()))
}

/// 符号付き。Plex Sans JP に U+2212 が無いので、ASCII の +/- を使う。
fn signed(value: Option<f64>, decimals: usize) -> String {
    value.map_or("—".to_owned(), |v| format!("{v:+.decimals$}"))
}

/// 3 桁ごとに区切る。
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn hours_minutes(minutes: f64) -> (String, String) {
    let total = minutes.max(0.0).round() as u64;
    ((total / 60).to_string(), (total % 60).to_string())
}

fn local(ms: i64) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local))
}

/// `HH:MM`（ローカル時刻）。
fn time(ms: i64) -> String {
    local(ms).map_or("—".to_owned(), |d| d.format("%H:%M").to_string())
}

fn time_opt(ms: Option<i64>) -> String {
    ms.map_or("—".to_owned(), time)
}

/// 「10/1（木）の夜」。
fn night_label(ms: i64) -> String {
    use chrono::Datelike;
    let Some(d) = local(ms) else {
        return "—".to_owned();
    };
    let weekday =
        ["月", "火", "水", "木", "金", "土", "日"][d.weekday().num_days_from_monday() as usize];
    format!("{}/{}（{weekday}）の夜", d.month(), d.day())
}

/// その日の 0:00（ローカル）のタイムスタンプ。
fn local_midnight(ms: i64) -> i64 {
    local(ms)
        .and_then(|d| d.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|d| d.and_local_timezone(chrono::Local).earliest())
        .map_or(ms, |d| d.timestamp_millis())
}

/// 今日を 0 とした日数（昨日が -1）。
fn days_ago(ms: i64) -> f64 {
    let today = chrono::Local::now().date_naive();
    local(ms).map_or(0.0, |d| (d.date_naive() - today).num_days() as f64)
}

/// 正午からの分。日付をまたぐ時刻（23:50 と 00:10）を平均できるようにする。
fn minutes_from_noon(ms: i64) -> f64 {
    use chrono::Timelike;
    local(ms).map_or(0.0, |d| {
        let minutes = f64::from(d.hour() * 60 + d.minute());
        (minutes - 720.0).rem_euclid(1440.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_on_missing_values_and_time_gaps() {
        let points = [
            (0.0, Some(1.0)),
            (1.0, Some(2.0)),
            (2.0, None),
            (3.0, Some(3.0)),
            (9.0, Some(4.0)),
        ];
        let r = runs(points.into_iter(), 1.5);
        assert_eq!(
            r,
            vec![
                vec![[0.0, 1.0], [1.0, 2.0]],
                vec![[3.0, 3.0]],
                vec![[9.0, 4.0]]
            ]
        );
        assert_eq!(gaps(&r, 0.0..=9.0), vec![1.0..=3.0, 3.0..=9.0]);
    }

    #[test]
    fn numbers_are_grouped_and_missing_values_are_dashes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(12_480), "12,480");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(opt(None::<f64>, 0), "—");
        assert_eq!(opt(Some(61.6), 0), "62");
        assert_eq!(signed(Some(-3.2), 0), "-3");
        assert_eq!(hours_minutes(348.4), ("5".to_owned(), "48".to_owned()));
    }

    #[test]
    fn settings_changes_are_counted() {
        let saved = Settings::default();
        let mut draft = saved.clone();
        assert_eq!(changes(&draft, &saved), 0);
        draft.osc_enabled = true;
        draft.osc_port = 9001;
        assert_eq!(changes(&draft, &saved), 2);
        draft.oyasumi_enabled = true;
        draft.oyasumi_allow_hr_only = true;
        draft.oyasumi_query_port = 12345;
        assert_eq!(changes(&draft, &saved), 5);
    }
    #[test]
    fn daily_graph_includes_all_segments_and_active_state() {
        let mut a = sleep_engine::SessionTracker::new(1000).session;
        a.sleep_day = Some("2026-10-02".into());
        a.ended_at = Some(2000);
        let mut b = sleep_engine::SessionTracker::new(3000).session;
        b.sleep_day = Some("2026-10-02".into());
        let mut c = sleep_engine::SessionTracker::new(86400000).session;
        c.sleep_day = Some("2026-10-03".into());
        let raw = vec![a.clone(), b.clone(), c.clone()];
        let days = storage::group_sleep_days(&raw, 12);
        let history = raw
            .iter()
            .map(|s| FeatureMinute {
                timestamp: s.started_at,
                features: Features::default(),
                sleep_probability: Some(0.9),
                session_id: Some(s.session_id.clone()),
                sample_count: 60,
            })
            .collect();
        let v = View {
            days,
            raw_sessions: raw,
            history,
            active: Some(b),
            ..Default::default()
        };
        let minutes = session_minutes(&v, "day:2026-10-02");
        assert_eq!(minutes.len(), 2);
        assert!(day_is_active(&v, "day:2026-10-02"));
        assert!(!day_is_active(&v, "day:2026-10-03"));
    }

    #[test]
    fn runtime_messages_are_localized() {
        assert_eq!(
            localize("Settings applied; persistence is asynchronous"),
            "設定を適用しました"
        );
        assert_eq!(
            localize("timeout; automatic retry in 3 s"),
            "timeout。3 秒後に再接続します"
        );
        assert_eq!(localize("something new"), "something new");
        assert_eq!(
            osc_state("UDP → 127.0.0.1:9000 (VRChat presence unverified)").1,
            "送信中"
        );
        assert_eq!(osc_state("Disabled").1, "無効");
    }
}

#[cfg(test)]
mod mode_render_tests {
    use super::*;
    fn rendered_text(rr: bool) -> String {
        let core = RealtimeCore::new();
        core.ingest(Measurement {
            timestamp: now_ms(),
            source_timestamp: None,
            sensor_id: "test".into(),
            heart_rate: 60,
            rr_ms: if rr { vec![1000.0] } else { vec![] },
            contact: None,
        });
        let view = Arc::new(Mutex::new(View::default()));
        view.lock().unwrap().display.push_back(core.latest());
        let (tx, _rx) = mpsc::channel(8);
        let mut app = Desktop::new(
            core,
            Arc::new(storage::Health::default()),
            view,
            tx,
            Settings::default(),
            true,
            "test".into(),
        );
        app.page = Page::Realtime;
        let ctx = egui::Context::default();
        theme::install(&ctx, theme::bundled_fonts());
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 3000.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input(), |ctx| app.draw(ctx));
        let frame = ctx.run(input(), |ctx| app.draw(ctx));
        fn texts(shape: &egui::epaint::Shape, out: &mut String) {
            match shape {
                egui::epaint::Shape::Text(t) => {
                    out.push_str(t.galley.text());
                    out.push('\n');
                }
                egui::epaint::Shape::Vec(v) => {
                    for s in v {
                        texts(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = String::new();
        for shape in frame.shapes {
            texts(&shape.shape, &mut out);
        }
        out
    }
    #[test]
    fn hr_only_has_no_rr_or_hrv_widgets() {
        let text = rendered_text(false);
        assert!(!text.contains("RR Interval"), "{text}");
        assert!(!text.contains("RMSSD"), "{text}");
        assert!(text.contains("擬似睡眠"), "{text}");
    }
    #[test]
    fn rr_mode_retains_rr_and_hrv_widgets() {
        let text = rendered_text(true);
        assert!(text.contains("RR Interval"), "{text}");
        assert!(text.contains("RMSSD"), "{text}");
    }
}
