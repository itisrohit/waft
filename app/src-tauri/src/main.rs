#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod daemon_client;

use std::process::Command;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewWindow, WindowEvent};

const WINDOW_LABEL: &str = "main";

#[tauri::command]
fn daemon_status() -> Result<daemon_client::DaemonStatus, String> {
    daemon_client::DaemonClient::from_environment().ensure_running()
}

#[tauri::command]
fn nearby_peers() -> Result<Vec<daemon_client::NearbyPeer>, String> {
    daemon_client::nearby_peers()
}

#[tauri::command]
fn device_name() -> String {
    std::env::var("WAFT_PEER_NAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| !name.trim().is_empty())
        })
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "This device".to_string())
}

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
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);

    // A visible but unfocused window may be behind another app. In that case,
    // the tray click should raise it instead of hiding it first.
    if visible && focused {
        if let Err(error) = window.hide() {
            eprintln!("waft: could not hide tray window: {error}");
        }
    } else {
        show_window(window);
    }
}

fn show_window(window: &WebviewWindow) {
    if let Err(error) = window.unminimize() {
        eprintln!("waft: could not restore tray window: {error}");
    }
    if let Err(error) = window.show() {
        eprintln!("waft: could not show tray window: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("waft: could not focus tray window: {error}");
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            nearby_peers,
            device_name
        ])
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
                            show_window(&window);
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
