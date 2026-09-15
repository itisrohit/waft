#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewWindow, WindowEvent};

const WINDOW_LABEL: &str = "main";

fn tray_image() -> Image<'static> {
    const SIDE: usize = 24;
    let mut rgba = vec![0_u8; SIDE * SIDE * 4];

    // Draw a compact white W with transparent surroundings for the tray.
    for x in 4..20 {
        let progress = x - 4;
        let y = if progress < 4 {
            7 + progress
        } else if progress < 8 {
            11 - (progress - 4)
        } else if progress < 12 {
            7 + (progress - 8)
        } else {
            11 - (progress - 12)
        };
        for dy in 0..3 {
            let py = y + dy;
            let index = (py * SIDE + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }

    Image::new_owned(rgba, SIDE as u32, SIDE as u32)
}

fn toggle_window(window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open waft", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit waft", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

            TrayIconBuilder::new()
                .icon(tray_image())
                .tooltip("waft")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                            toggle_window(&window);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                        && let Some(window) = tray.app_handle().get_webview_window(WINDOW_LABEL)
                    {
                        toggle_window(&window);
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == WINDOW_LABEL
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run waft desktop");
}
