mod paths;
mod plot;
mod runtime;
mod theme;
mod ui;
mod widgets;
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tungstenite=off".parse().unwrap())
                .add_directive("tokio_tungstenite=off".parse().unwrap()),
        )
        .init();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help") {
        println!("nemuribeat [--demo] [--demo-hr-only] [--demo-sleep] [--headless] [--seconds N] [--data-dir PATH] [--api] [--ipc PATH] [--pulsoid]\nDefault: native GUI; select a BLE sensor in Sensors. Demo uses a separate database.");
        return Ok(());
    }
    let value = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let demo_hr_only = args.iter().any(|a| a == "--demo-hr-only");
    let demo_sleep = args.iter().any(|a| a == "--demo-sleep");
    let demo = args.iter().any(|a| a == "--demo") || demo_hr_only || demo_sleep;
    let headless = args.iter().any(|a| a == "--headless");
    let seconds = value("--seconds").map(|x| x.parse::<u64>()).transpose()?;
    let explicit = value("--data-dir").map(PathBuf::from);
    let paths = paths::resolve(
        explicit.clone(),
        directories::BaseDirs::new().map(|d| d.data_local_dir().to_path_buf()),
        &std::env::current_exe()?,
        demo,
    )?;
    let dir = paths.root.clone();
    let database = paths.personal.clone();
    if !database.exists() {
        let filename = if demo { "demo.sqlite" } else { "physio.sqlite" };
        let legacy = explicit.as_ref().map(|p| p.join(filename)).or_else(|| {
            directories::ProjectDirs::from("dev", "VRC-Health-Check", "PhysioCore")
                .map(|p| p.data_local_dir().join(filename))
        });
        if let Some(source) = legacy.filter(|p| p.exists()) {
            storage::migrate_legacy(&source, &database)?;
        }
    }
    let (storage, worker) = storage::Storage::start(database.clone())?;
    let (cache, cache_worker) = storage::Storage::start(paths.cache.clone())?;
    let mut settings = storage::settings(&database)?;
    settings.validate().map_err(anyhow::Error::msg)?;
    if args.iter().any(|a| a == "--api") {
        settings.api_enabled = true;
    }
    let core = physio_core::RealtimeCore::new();
    let shared = Arc::new(Mutex::new(runtime::View::default()));
    let (commands, rx) = tokio::sync::mpsc::channel(32);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let backend = rt.spawn(runtime::run(
        core.clone(),
        storage.clone(),
        database.clone(),
        cache.clone(),
        paths.cache.clone(),
        settings.clone(),
        shared.clone(),
        rx,
        demo,
        demo_hr_only,
        demo_sleep,
        value("--ipc"),
    ));
    let mut gui_error = None;
    if headless {
        rt.block_on(async {
            if args.iter().any(|a| a == "--pulsoid") {
                let token = std::env::var("PULSOID_ACCESS_TOKEN").unwrap_or_default();
                let _ = commands.send(runtime::Command::ConnectPulsoid(token)).await;
            }
            if let Some(seconds) = seconds {
                tokio::time::sleep(Duration::from_secs(seconds)).await;
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        });
    } else {
        let app = ui::Desktop::new(
            core.clone(),
            storage.health.clone(),
            shared,
            commands.clone(),
            settings,
            demo,
            dir.display().to_string(),
        );
        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_icon(app_icon()?)
                .with_inner_size([1180.0, 820.0])
                .with_min_inner_size([960.0, 640.0]),
            ..Default::default()
        };
        if let Err(e) = eframe::run_native(
            "NemuriBeat",
            options,
            Box::new(|cc| {
                theme::install(&cc.egui_ctx, theme::bundled_fonts());
                cc.egui_ctx.set_theme(eframe::egui::ThemePreference::Dark);
                Ok(Box::new(app))
            }),
        ) {
            tracing::error!("GUI failed: {e}");
            gui_error = Some(anyhow::anyhow!("GUI failed: {e}"));
        }
    }
    let backend_result = rt.block_on(async {
        let _ = commands.send(runtime::Command::Shutdown).await;
        backend.await
    });
    let cache_result = cache_worker.shutdown();
    let storage_result = worker.shutdown();
    backend_result??;
    storage_result?;
    cache_result?;
    if let Some(error) = gui_error {
        return Err(error);
    }
    println!(
        "Stopped cleanly. sequence={} stored={} dropped={} database={}",
        core.latest().sequence_id,
        storage
            .health
            .written
            .load(std::sync::atomic::Ordering::Relaxed),
        storage
            .health
            .dropped
            .load(std::sync::atomic::Ordering::Relaxed),
        database.display()
    );
    rt.shutdown_timeout(Duration::from_secs(3));
    Ok(())
}

fn app_icon() -> anyhow::Result<eframe::egui::IconData> {
    let rgba = image::load_from_memory(include_bytes!(
        "../assets/branding/nemuribeat-appicon-256.png"
    ))?
    .into_rgba8();
    Ok(eframe::egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}
