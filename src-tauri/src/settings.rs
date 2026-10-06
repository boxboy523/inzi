use std::fs;

use tauri::{AppHandle, Manager};

use crate::config::AppConfig;

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "settings window not found".to_string())?;

    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
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
