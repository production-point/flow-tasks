use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Runtime,
};

use crate::show_and_focus;

pub fn create_tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // Right-click menu — the guaranteed escape hatch. If the hotkey and
    // left-click toggle both fail (e.g., off-screen window), the user can
    // still right-click the tray and pick "Show".
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;

    TrayIconBuilder::with_id("main")
        .tooltip("Flow Tasks")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    show_and_focus(&window);
                }
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    // Toggle, but if the window is "visible" yet off-screen,
                    // treat that as hidden so we summon it back.
                    let visible = window.is_visible().unwrap_or(false);
                    let minimized = window.is_minimized().unwrap_or(false);
                    if visible && !minimized && window_is_on_screen(&window) {
                        let _ = window.hide();
                    } else {
                        show_and_focus(&window);
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// Duplicate of the private helper in `lib.rs`, kept crate-local so the tray
/// module doesn't pull everything in. Minimal by design.
fn window_is_on_screen<R: Runtime>(window: &tauri::WebviewWindow<R>) -> bool {
    const MIN_VISIBLE_PX: i32 = 50;
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
        let overlap_x = (wx + ww).min(mp.x + ms.width as i32) - wx.max(mp.x);
        let overlap_y = (wy + wh).min(mp.y + ms.height as i32) - wy.max(mp.y);
        overlap_x >= MIN_VISIBLE_PX && overlap_y >= MIN_VISIBLE_PX
    })
}
