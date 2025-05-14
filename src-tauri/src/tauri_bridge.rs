use tauri::{AppHandle, Emitter, Manager};

use crate::domain::TelemetryEvent;

pub fn emit_event(app: &AppHandle, event: &TelemetryEvent) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("telemetry", event);
    }
}
