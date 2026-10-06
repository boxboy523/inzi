use std::fs;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::config::AppConfig;

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App("settings.html".into()),
    )
    .title("INZI 설정")
    .inner_size(820.0, 760.0)
    .min_inner_size(640.0, 520.0)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn get_config_json() -> Result<String, String> {
    match fs::read_to_string("config.json") {
        Ok(content) => Ok(content),
        Err(_) => serde_json::to_string_pretty(&AppConfig::default()).map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn save_config_json(config_json: String) -> Result<(), String> {
    let config: AppConfig = serde_json::from_str(&config_json)
        .map_err(|e| format!("JSON 또는 설정 형식 오류: {}", e))?;

    match config.gauge.model.as_str() {
        "4G700" | "4G710" => {}
        other => return Err(format!("지원하지 않는 측정기 기종: {}", other)),
    }

    hex::decode(&config.gauge.read_req_hex)
        .map_err(|e| format!("read_req_hex 오류: {}", e))?;
    hex::decode(&config.gauge.write_req_hex_0)
        .map_err(|e| format!("write_req_hex_0 오류: {}", e))?;
    hex::decode(&config.gauge.write_req_hex)
        .map_err(|e| format!("write_req_hex 오류: {}", e))?;

    let pretty = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write("config.json", pretty).map_err(|e| format!("config.json 저장 실패: {}", e))
}
