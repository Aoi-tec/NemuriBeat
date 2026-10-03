//! Native tray lifecycle. Closing the window hides it only when a tray can restore it.
use eframe::egui::{self, ViewportCommand};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Default)]
struct State {
    hidden: AtomicBool,
    quitting: AtomicBool,
    overlay_url: std::sync::Mutex<String>,
}
#[derive(Default)]
pub struct Resident {
    attempted: bool,
    start_hidden: bool,
    state: Arc<State>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    native: Option<Native>,
}
#[derive(PartialEq, Debug)]
enum CloseAction {
    Hide,
    Exit,
}
fn close_action(available: bool, close_to_tray: bool, quitting: bool) -> CloseAction {
    if available && close_to_tray && !quitting {
        CloseAction::Hide
    } else {
        CloseAction::Exit
    }
}
impl Resident {
    pub fn start_hidden(&mut self, value: bool) {
        self.start_hidden = value;
    }
    pub fn available(&self) -> bool {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            self.native.is_some()
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            false
        }
    }
    pub fn hide(&self, ctx: &egui::Context) {
        if self.available() {
            self.state.hidden.store(true, Ordering::Relaxed);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
    }
    pub fn quit(&self, ctx: &egui::Context) {
        self.state.quitting.store(true, Ordering::Relaxed);
        ctx.send_viewport_cmd(ViewportCommand::Close);
        ctx.request_repaint();
    }
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        close_to_tray: bool,
        overlay_url: String,
    ) -> Option<String> {
        *self.state.overlay_url.lock().unwrap() = overlay_url.clone();
        let mut error = None;
        if !self.attempted {
            self.attempted = true;
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            match Native::new(ctx.clone(), self.state.clone()) {
                Ok(native) => self.native = Some(native),
                Err(e) => {
                    error = Some(format!(
                        "トレイを作成できません。ウィンドウを閉じると終了します: {e}"
                    ))
                }
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let _ = overlay_url;
            if self.start_hidden {
                self.hide(ctx);
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && close_action(
                self.available(),
                close_to_tray,
                self.state.quitting.load(Ordering::Relaxed),
            ) == CloseAction::Hide
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        error
    }
    pub fn visible(&self) -> bool {
        !self.state.hidden.load(Ordering::Relaxed)
    }
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
struct Native {
    _icon: tray_icon::TrayIcon,
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Native {
    fn new(ctx: egui::Context, state: Arc<State>) -> anyhow::Result<Self> {
        use tray_icon::{
            menu::{Menu, MenuEvent, MenuItem},
            MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
        };
        let menu = Menu::new();
        let open = MenuItem::new("NemuriBeatを表示", true, None);
        let hide = MenuItem::new("ウィンドウを隠す", true, None);
        let web = MenuItem::new("心拍Webページを開く", true, None);
        let quit = MenuItem::new("終了（計測を停止して保存）", true, None);
        menu.append_items(&[&open, &hide, &web, &quit])?;
        let rgba = image::load_from_memory(include_bytes!(
            "../assets/branding/nemuribeat-appicon-32.png"
        ))?
        .into_rgba8();
        let icon = tray_icon::Icon::from_rgba(rgba.as_raw().clone(), rgba.width(), rgba.height())?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("NemuriBeat · 計測とWeb配信を継続")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        let open_id = open.id().clone();
        let hide_id = hide.id().clone();
        let web_id = web.id().clone();
        let quit_id = quit.id().clone();
        let c = ctx.clone();
        let s = state.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                show(&c, &s);
            } else if event.id == hide_id {
                s.hidden.store(true, Ordering::Relaxed);
                c.send_viewport_cmd(ViewportCommand::Visible(false));
            } else if event.id == web_id {
                c.open_url(egui::OpenUrl::new_tab(
                    s.overlay_url.lock().unwrap().clone(),
                ));
            } else if event.id == quit_id {
                s.quitting.store(true, Ordering::Relaxed);
                c.send_viewport_cmd(ViewportCommand::Close);
            }
            c.request_repaint();
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(&ctx, &state);
            }
        }));
        Ok(Self { _icon: tray })
    }
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn show(ctx: &egui::Context, state: &State) {
    state.hidden.store(false, Ordering::Relaxed);
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Drop for Native {
    fn drop(&mut self) {
        tray_icon::menu::MenuEvent::set_event_handler(None::<fn(tray_icon::menu::MenuEvent)>);
        tray_icon::TrayIconEvent::set_event_handler(None::<fn(tray_icon::TrayIconEvent)>);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_hides_only_when_restoration_is_possible() {
        assert_eq!(close_action(true, true, false), CloseAction::Hide);
        assert_eq!(close_action(false, true, false), CloseAction::Exit);
        assert_eq!(close_action(true, false, false), CloseAction::Exit);
        assert_eq!(close_action(true, true, true), CloseAction::Exit);
    }
}
