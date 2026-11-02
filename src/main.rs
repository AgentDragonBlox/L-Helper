// LHelper (Rust): lightweight control tool for Lenovo LOQ, Legion and IdeaPad laptops.
// UI concept from G-Helper, hardware knowledge from Lenovo Legion Toolkit (both GPL-3.0),
// 24-zone lighting from LOQ-24-zone-RGB-controller (MIT).

// Release builds are a GUI app without a console window
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod automation;
mod config;
mod gpu;
mod lenovo;
mod lighting;
mod logo;
mod log;
mod mode;
mod nvapi;
mod tray;
mod ui;
mod updates;
mod win;
mod wmi;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenProcess, SetEvent, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};

/// Taskbar ID of the window and the Start menu shortcut (win/package.rs)
pub const AUMID: &str = "LHelper.LHelper";

fn main() {
    // Before anything loads a DLL or touches settings (win/secure.rs)
    win::secure::harden_process();
    log::init_data_dir();

    let args: Vec<String> = std::env::args().collect();
    let screenshot_dir = args.iter().position(|a| a == "-screenshot").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let screenshots = screenshot_dir.is_some();
    let simulate_ideapad = args.iter().any(|a| a == "-simulate") && args.iter().any(|a| a == "ideapad");
    let autostart = args.iter().any(|a| a == "-autostart");
    // Started by the Windows startup task: stay in the tray unless "Start minimized to tray" is off
    let start_hidden = autostart && config::is_not_false("start_minimized");

    log!("------------ LHelper (Rust) {} {}", env!("CARGO_PKG_VERSION"), args[1..].join(" "));

    // Run by the MSI installer: do the one step and exit, no UI
    if let Some(code) = win::package::installer_command(&args) {
        std::process::exit(code);
    }

    // Single instance: a second launch asks the first one to show itself and waits for it to confirm.
    // If the running copy doesn't answer (hung, or stuck shutting down inside a driver call), end it and start anyway,
    // so LHelper can never get locked out by a dead instance.
    unsafe {
        // Restarted by the previous instance (win/package.rs): let it exit first
        if let Some(pid) = args.iter().position(|a| a == "-after").and_then(|i| args.get(i + 1)).and_then(|p| p.parse::<u32>().ok()) {
            if let Ok(old) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                WaitForSingleObject(old, 10_000);
                let _ = windows::Win32::Foundation::CloseHandle(old);
            }
        }
        let mutex = CreateMutexW(None, true, w!("Global\\LHelperRsApp"));
        if GetLastError() == ERROR_ALREADY_EXISTS && !screenshots {
            let show = CreateEventW(None, false, false, w!("Global\\LHelperRsApp-Show"));
            let ack = CreateEventW(None, false, false, w!("Global\\LHelperRsApp-Ack"));
            if let (Ok(show), Ok(ack)) = (show, ack) {
                let _ = SetEvent(show);
                if WaitForSingleObject(ack, 3000) == WAIT_OBJECT_0 {
                    log!("Another LHelper is running, asked it to show its window");
                    return;
                }
            }
            log!("Running LHelper didn't respond, replacing it");
            win::terminate_other_instances();
        }
        std::mem::forget(mutex);
    }

    // First start after the MSI: register for Dynamic Lighting, then hand over to a copy with the package identity
    if !screenshots && win::package::register_after_install() {
        return;
    }

    // Hardware detection on a plain MTA thread (the UI thread belongs to winit / OLE)
    let platform = std::thread::spawn(move || {
        wmi::init_security();
        let m = lenovo::machine::detect(simulate_ideapad);
        if !simulate_ideapad {
            // Screenshot mode stays read-only: detect the keyboard but don't take over lighting
            if screenshots {
                lighting::detect();
            } else {
                lighting::init();
            }
        }
        m.platform
    })
    .join()
    .unwrap_or(lenovo::machine::Platform::Unknown);

    if platform == lenovo::machine::Platform::Unknown {
        win::message_box(
            "No supported Lenovo control interface was found.\n\nLOQ / Legion need the GameZone WMI interface (\"Lenovo Vantage Gaming Feature Driver\"). IdeaPad / Yoga / ThinkBook need the Lenovo energy driver and \"Lenovo Process Management\".\n\nThe app will start, but most controls won't work.",
            false,
        );
    }

    if !screenshots {
        app::start_background();
        // Quit from the tray must work while the window is hidden (no UI frames run then), so exit right here
        tray::start(toggle_window, || app::quit());
        // Re-show requests from second launches
        std::thread::spawn(|| unsafe {
            let ack = CreateEventW(None, false, false, w!("Global\\LHelperRsApp-Ack"));
            if let Ok(event) = CreateEventW(None, false, false, w!("Global\\LHelperRsApp-Show")) {
                loop {
                    WaitForSingleObject(event, INFINITE);
                    win::show_near_tray(app::hwnd());
                    app::repaint();
                    if let Ok(ack) = &ack {
                        let _ = SetEvent(*ack);
                    }
                }
            }
        });
    } else {
        app::job(|| {
            app::refresh_state();
            app::refresh_sensors();
        });
    }

    // Own taskbar identity, so the button shows the window's icon. Under the Dynamic Lighting package the
    // taskbar otherwise looks the logo up in the package (no app list entry), and finds a blank square.
    // Package identity (and so Dynamic Lighting) comes from the process token, not this ID.
    // (win::set_window_app_id sets it on the window too, which packaged processes need)
    if let Err(e) = unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(AUMID)) } {
        log!("Taskbar: couldn't set the process ID: {}", e.message());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // G-Helper style title bar: name and model, close button only (close hides to the tray)
            .with_title(format!("L-Helper — {}", lenovo::machine::get().model))
            .with_inner_size([ui::WIDTH, 700.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_minimize_button(false)
            .with_visible(!start_hidden)
            .with_icon(egui::IconData { rgba: logo::rgba(logo::Logo::Blue, 64), width: 64, height: 64 }),
        ..Default::default()
    };

    let result = eframe::run_native(
        "LHelper",
        options,
        Box::new(move |cc| {
            let app = ui::LHelperApp::new(cc, screenshot_dir);
            if !start_hidden {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    win::show_near_tray(app::hwnd());
                });
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        log!("UI failed: {e}");
    }
    app::shutdown();
}

/// Tray left click: show next to the tray, or hide
fn toggle_window() {
    let hwnd = app::hwnd();
    if win::is_visible(hwnd) {
        win::show_window(hwnd, false);
    } else {
        win::show_near_tray(hwnd);
        app::job(app::refresh_state);
    }
    app::repaint();
}
