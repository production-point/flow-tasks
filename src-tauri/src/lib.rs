mod commands;
mod tray;

use tauri::{Manager, Runtime, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

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
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
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

            // Register global hotkey Ctrl+Alt+T
            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT);
            app.global_shortcut().register(shortcut)?;

            // Start hidden in tray
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
