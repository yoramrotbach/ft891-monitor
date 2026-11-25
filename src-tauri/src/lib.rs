//! 891 Monitor: a desktop CAT display and remote panel for the Yaesu FT-891.

pub use ft891_cat::{cat, sim, transport};
pub mod poller;
pub mod state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Monitor {
    worker: Mutex<Option<(Arc<AtomicBool>, JoinHandle<()>)>>,
    controls: Mutex<Option<Sender<cat::Control>>>,
}

impl Monitor {
    fn stop(&self) {
        *self.controls.lock().unwrap() = None;
        if let Some((flag, handle)) = self.worker.lock().unwrap().take() {
            flag.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
    }
}

#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(&app, "settings", tauri::WebviewUrl::App("settings.html".into()))
        .title("891 Monitor — Settings")
        .inner_size(460.0, 660.0)
        .min_inner_size(380.0, 360.0)
        .resizable(true)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_ports() -> Vec<transport::PortInfo> {
    transport::list_ports()
}

#[tauri::command]
fn list_modes() -> Vec<&'static str> {
    cat::ALL_MODES.iter().map(|m| m.lcd_label()).collect()
}

#[tauri::command]
async fn start_monitor(app: AppHandle, monitor: State<'_, Monitor>, config: poller::MonitorConfig) -> Result<(), String> {
    monitor.stop();
    let flag = Arc::new(AtomicBool::new(false));
    let f2 = flag.clone();
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("cat-poller".into())
        .spawn(move || {
            poller::run(config, f2, rx, |st| {
                let _ = app.emit("rig-state", st);
            })
        })
        .map_err(|e| e.to_string())?;
    *monitor.worker.lock().unwrap() = Some((flag, handle));
    *monitor.controls.lock().unwrap() = Some(tx);
    Ok(())
}

#[tauri::command]
async fn stop_monitor(monitor: State<'_, Monitor>) -> Result<(), String> {
    monitor.stop();
    Ok(())
}

/// Queue a control request. The poller validates it (allow-list, transmit
/// and lock checks) and reports refusals in `RigState::control_note`.
#[tauri::command]
fn rig_control(monitor: State<'_, Monitor>, action: cat::Control) -> Result<(), String> {
    action.command()?; // reject obviously invalid requests immediately
    match monitor.controls.lock().unwrap().as_ref() {
        Some(tx) => tx.send(action).map_err(|_| "Not connected".to_string()),
        None => Err("Not connected".into()),
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(Monitor::default())
        .invoke_handler(tauri::generate_handler![
            list_ports,
            list_modes,
            start_monitor,
            stop_monitor,
            open_settings,
            rig_control
        ])
        .setup(|app| {
            if std::env::args().any(|a| a == "--demo") {
                let handle = app.handle().clone();
                let cfg = poller::MonitorConfig { source: poller::Source::Demo, allow_control: true };
                tauri::async_runtime::spawn(async move {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let state = handle.state::<Monitor>();
                    let _ = start_monitor(handle.clone(), state, cfg).await;
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let app = window.app_handle();
                match window.label() {
                    "main" => {
                        app.state::<Monitor>().stop();
                        if let Some(w) = app.get_webview_window("settings") {
                            let _ = w.close();
                        }
                    }
                    "settings" => {
                        let _ = app.emit("settings-closed", ());
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running 891 Monitor");
}
