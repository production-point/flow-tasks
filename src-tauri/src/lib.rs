mod commands;
mod tray;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::{Manager, Runtime, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Shared "pinned" flag, written by the `toggle_pin` command and read by the
/// macOS hide-on-blur handler. When pinned the window is a torn-off,
/// always-on-top floating panel that must NOT auto-hide; when not pinned it
/// behaves as a menu-bar dropdown that hides on focus loss.
pub struct PinState(pub Arc<AtomicBool>);

/// Timestamp of the last macOS auto-hide (focus-loss). The tray left-click
/// handler reads it to break the dropdown double-toggle race: clicking the
/// menu-bar icon to dismiss an open panel first fires `Focused(false)` (which
/// hides the window), so the click would otherwise immediately re-summon it.
/// If a hide happened in the last few hundred ms, the tray click is treated as
/// "leave it closed".
pub struct LastHide(pub std::sync::Mutex<Option<std::time::Instant>>);

/// Minimum visible area, per axis, for the window to count as "on screen".
/// Mirrors `MIN_VISIBLE_PX` on the TS side — a sliver isn't enough.
const MIN_VISIBLE_PX: i32 = 50;

/// Force the main window visible, unminimized, focused, and on a real monitor.
/// The only escape hatch out of the "window is hidden off-screen" bug class —
/// safe to call no matter what state the window is in.
pub fn show_and_focus<R: Runtime>(window: &WebviewWindow<R>) {
    // If the window's outer rect doesn't overlap any available monitor by at
    // least MIN_VISIBLE_PX on both axes, recenter on the primary monitor
    // before showing. Otherwise the user sees nothing and has no way to drag.
    if !window_is_on_screen(window) {
        let _ = window.center();
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn window_is_on_screen<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    let Ok(pos) = window.outer_position() else {
        return false;
    };
    let Ok(size) = window.outer_size() else {
        return false;
    };
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    if monitors.is_empty() {
        return false;
    }

    let wx = pos.x;
    let wy = pos.y;
    let ww = size.width as i32;
    let wh = size.height as i32;

    monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        let mx = mp.x;
        let my = mp.y;
        let mw = ms.width as i32;
        let mh = ms.height as i32;
        let overlap_x = (wx + ww).min(mx + mw) - wx.max(mx);
        let overlap_y = (wy + wh).min(my + mh) - wy.max(my);
        overlap_x >= MIN_VISIBLE_PX && overlap_y >= MIN_VISIBLE_PX
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PinState(Arc::new(AtomicBool::new(false))))
        .manage(LastHide(std::sync::Mutex::new(None)))
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(window) = app.get_webview_window("main") {
                            // Global hotkey is a rescue — always summon, never hide.
                            // Prior behaviour toggled on is_visible(), which treats
                            // an off-screen window as "visible" and hid it again,
                            // making the app unreachable.
                            show_and_focus(&window);
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::toggle_pin,
            commands::store_api_key,
            commands::get_api_key,
            commands::delete_api_key,
        ])
        .setup(|app| {
            tray::create_tray(app.handle())?;

            // Live in the menu bar, not the Dock / ⌘-Tab switcher — the macOS
            // idiom for a tray-anchored companion. Windows is unaffected.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Register the summon hotkey. macOS uses Cmd+Option+T (Ctrl+Alt+T
            // collides with native shortcuts and isn't idiomatic); every other
            // platform keeps Ctrl+Alt+T.
            #[cfg(target_os = "macos")]
            let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::ALT), Code::KeyT);
            #[cfg(not(target_os = "macos"))]
            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT);
            app.global_shortcut().register(shortcut)?;

            // Start hidden in tray
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();

                // macOS dropdown behaviour: hide on focus loss, but only when
                // not pinned (a torn-off floating panel stays put). Windows
                // keeps its explicit tray-toggle / hotkey model.
                #[cfg(target_os = "macos")]
                {
                    let app_handle = app.handle().clone();
                    let win = window.clone();
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::Focused(false) = event {
                            use std::sync::atomic::Ordering;
                            let pinned = app_handle.state::<PinState>().0.load(Ordering::Relaxed);
                            if !pinned {
                                if let Ok(mut last) = app_handle.state::<LastHide>().0.lock() {
                                    *last = Some(std::time::Instant::now());
                                }
                                let _ = win.hide();
                            }
                        }
                    });
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
