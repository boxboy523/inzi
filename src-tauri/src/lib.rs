use std::sync::{Arc, OnceLock};
use std::{collections::HashMap, sync::Mutex};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{Manager, State};

use crate::cnc::{update_offset_logs, ToolData};
use crate::logger::HistoryLogger;
use crate::{
    cnc::spawn_cnc_loop, config::AppConfig, fwlib::FocasClient, gauge::spawn_gauge_stream,
};

pub mod cnc;
pub mod config;
pub mod fwlib;
pub mod gauge;
pub mod logger;

#[derive(Debug, Clone)]
pub struct HexCommands {
    pub read_req_hex: Vec<u8>,
    pub write_req_hex_0: Vec<u8>,
    pub write_req_hex_1: Vec<u8>,
}

static HEX_CMDS: OnceLock<HexCommands> = OnceLock::new();

pub struct AppState {
    pub handle_table: Arc<HashMap<u16, FocasClient>>,
    pub tool_data: Arc<Mutex<HashMap<u16, (ToolData, ToolData)>>>,
    pub batch_size: Arc<Mutex<HashMap<u16, usize>>>,
    pub password: String,
    pub log_path: String,
    pub font_size: u32,
}

#[derive(Debug, Serialize)]
pub struct OffsetLog {
    pub timestamp: DateTime<Utc>,
    pub machine_id: u16,
    pub tool_num: i16,
    pub old_value: i32,
    pub change_amount: i32,
    pub new_value: i32,
    pub success: bool,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ToolUiState {
    #[serde(flatten)]
    pub data: ToolData,
    pub current_offset: f64,
    pub previous_offset: f64,
    pub life: i16,
    pub count: i16,
}

#[derive(Debug, Serialize, Clone)]
pub struct MachineUiState {
    pub machine_id: u16,
    pub upper_tool: ToolUiState, // 황삭 (Tuple의 0번)
    pub lower_tool: ToolUiState, // 정삭 (Tuple의 1번)
    pub batch_size: usize,
}

#[tauri::command]
fn verify_password(input: String, state: State<'_, AppState>) -> bool {
    input == state.password
}

#[tauri::command]
async fn get_offset_history(
    machine_id: u16,
    tool_num: i16,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<OffsetLog>, String> {
    HistoryLogger::get_history(state.log_path.clone(), machine_id, tool_num, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_latest_offset_log(
    machine_id: u16,
    tool_num: i16,
    state: tauri::State<'_, AppState>,
) -> Result<Option<OffsetLog>, String> {
    HistoryLogger::get_latest_log(state.log_path.clone(), machine_id, tool_num)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_all_machine_states(state: State<'_, AppState>) -> Result<Vec<MachineUiState>, String> {
    let tool_data_map = state.tool_data.lock().unwrap().clone();
    let batch_size_map = state.batch_size.lock().unwrap().clone();

    let mut results = Vec::new();
    let mut keys: Vec<u16> = tool_data_map.keys().cloned().collect();
    keys.sort();

    let handle_table = state.handle_table.clone();

    for id in keys {
        if let Some((upper, lower)) = tool_data_map.get(&id) {
            let size = *batch_size_map.get(&id).unwrap_or(&5);

            let upper_log =
                HistoryLogger::get_latest_log(state.log_path.clone(), id, upper.tool_num)
                    .await
                    .map_err(|e| e.to_string())?;
            let lower_log =
                HistoryLogger::get_latest_log(state.log_path.clone(), id, lower.tool_num)
                    .await
                    .map_err(|e| e.to_string())?;

            let client = handle_table
                .get(&id)
                .ok_or_else(|| format!("No CNC client found for machine {}", id))?;

            let upper_life = client.read_life(upper.tool_num).unwrap_or(-1);
            let lower_life = client.read_life(lower.tool_num).unwrap_or(-1);
            let upper_count = client.read_count(upper.tool_num).unwrap_or(-1);
            let lower_count = client.read_count(lower.tool_num).unwrap_or(-1);

            let upper_ui = ToolUiState {
                data: upper.clone(),
                current_offset: client
                    .rdtofs(upper.tool_num, 0)
                    .map(|v| v.data as f64 / 10000.0)
                    .unwrap_or(0.0),
                previous_offset: upper_log
                    .as_ref()
                    .map_or(0.0, |log| log.old_value as f64 / 10000.0),
                life: upper_life,
                count: upper_count,
            };

            let lower_ui = ToolUiState {
                data: lower.clone(),
                current_offset: client
                    .rdtofs(lower.tool_num, 0)
                    .map(|v| v.data as f64 / 10000.0)
                    .unwrap_or(0.0),
                previous_offset: lower_log
                    .as_ref()
                    .map_or(0.0, |log| log.old_value as f64 / 10000.0),
                life: lower_life,
                count: lower_count,
            };

            results.push(MachineUiState {
                machine_id: id,
                upper_tool: upper_ui,
                lower_tool: lower_ui,
                batch_size: size,
            });
        }
    }
    Ok(results)
}

#[tauri::command]
async fn update_tool_settings(
    machine_id: u16,
    is_upper: bool, // true: 황삭, false: 정삭
    basic_size: Option<f64>,
    manual_offset: Option<f64>,
    offset_rate: Option<f64>,
    active: Option<bool>,
    tool_num: Option<i16>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut tool_data_map = state.tool_data.lock().unwrap();

    if let Some((upper, lower)) = tool_data_map.get_mut(&machine_id) {
        let target_tool = if is_upper { upper } else { lower };

        if let Some(v) = basic_size {
            target_tool.basic_size = v;
        }
        if let Some(v) = manual_offset {
            target_tool.manual_offset = v;
        }
        if let Some(v) = offset_rate {
            target_tool.offset_rate = v;
        }
        if let Some(v) = active {
            target_tool.active = v;
        }
        if let Some(v) = tool_num {
            target_tool.tool_num = v;
        }

        let mut config = AppConfig::load("config.json");
        config.update_from_state(&state);
        if let Err(e) = config.save("config.json") {
            eprintln!("Config save failed: {}", e);
        }

        Ok(())
    } else {
        Err("Machine ID not found".to_string())
    }
}

#[tauri::command]
async fn update_batch_size(
    machine_id: u16,
    new_size: usize,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut batch_map = state.batch_size.lock().unwrap();
    if new_size > 30 {
        return Err("Max batch size is 30".to_string());
    }
    batch_map.insert(machine_id, new_size);
    let mut config = AppConfig::load("config.json");
    config.update_from_state(&state);
    if let Err(e) = config.save("config.json") {
        eprintln!("Config save failed: {}", e);
    }
    Ok(())
}

#[tauri::command]
fn get_font_size(state: State<'_, AppState>) -> u32 {
    state.font_size
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "linux")]
            {
                let log_file = std::ffi::CString::new("focas2.log").unwrap();
                unsafe {
                    use crate::fwlib::cnc_startupprocess;
                    cnc_startupprocess(3, log_file.as_ptr())
                };
            }
            let config = AppConfig::load("config.json");
            let mut handle_table = HashMap::new();
            for machine in &config.machines {
                match FocasClient::new(&machine.ip, machine.port as i16, 10) {
                    Ok(client) => {
                        println!(
                            "Connected to CNC {} at {}:{}",
                            machine.name, machine.ip, machine.port
                        );
                        handle_table.insert(machine.id as u16, client);
                    }
                    Err(e) => {
                        println!(
                            "Failed to connect to CNC {} at {}:{} - {}",
                            machine.name, machine.ip, machine.port, e
                        );
                    }
                }
            }
            let hex_cmds = HexCommands {
                read_req_hex: hex::decode(&config.gauge.read_req_hex)
                    .expect("Invalid read_req_hex in config"),
                write_req_hex_0: hex::decode(&config.gauge.write_req_hex_0)
                    .expect("Invalid write_req_hex_0 in config"),
                write_req_hex_1: hex::decode(&config.gauge.write_req_hex_1)
                    .expect("Invalid write_req_hex_1 in config"),
            };
            HEX_CMDS.set(hex_cmds).unwrap_or_else(|_| {
                panic!("Failed to set HEX_CMDS from config. This should never happen since it's only set once.")
            });
            let (gauge_tx, gauge_rx) = tokio::sync::broadcast::channel(100);
            let handle_table = Arc::new(handle_table);
            let history_logger = Arc::new(HistoryLogger::new(&config.log_path));
            let app_state = AppState {
                handle_table: handle_table.clone(),
                tool_data: Arc::new(Mutex::new(config.mapping.tool_data.clone())),
                batch_size: Arc::new(Mutex::new(config.mapping.batch_size)),
                password: config.admin.password.clone(),
                log_path: config.log_path.clone(),
                font_size: config.ui.font_size,
            };
            let handle_table_clone = Arc::clone(&app_state.handle_table);
            let tool_data_clone = Arc::clone(&app_state.tool_data);
            let batch_size_clone = Arc::clone(&app_state.batch_size);
            let history_logger_clone = Arc::clone(&history_logger);
            tauri::async_runtime::spawn(async move {
                match spawn_cnc_loop(
                    gauge_rx,
                    handle_table_clone,
                    tool_data_clone,
                    batch_size_clone,
                    history_logger_clone,
                ) {
                    Ok(_) => println!("CNC loop exited gracefully"),
                    Err(e) => eprintln!("CNC loop encountered an error: {}", e),
                };
            });

            let handle_table_clone = Arc::clone(&app_state.handle_table);
            let tool_data_clone = Arc::clone(&app_state.tool_data);
            let history_logger_clone = Arc::clone(&history_logger);
            tauri::async_runtime::spawn(async move {
                update_offset_logs(history_logger_clone, handle_table_clone, tool_data_clone).await;
            });

            tauri::async_runtime::spawn(async move {
                match spawn_gauge_stream(
                    &config.gauge.ip,
                    config.gauge.port,
                    gauge_tx,
                ) {
                    Ok(_) => println!("Gauge stream exited gracefully"),
                    Err(e) => eprintln!("Gauge stream encountered an error: {}", e),
                };
            });
            app.manage(app_state);
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let app_handle = window.app_handle();
                if let Some(state) = app_handle.try_state::<AppState>() {
                    let mut config = AppConfig::load("config.json");
                    state
                        .tool_data
                        .lock()
                        .unwrap()
                        .iter_mut()
                        .for_each(|(_, (upper, lower))| {
                            upper.active = false;
                            lower.active = false;
                        });
                    config.update_from_state(&state);
                    if let Err(e) = config.save("config.json") {
                        eprintln!("Failed to save config: {}", e);
                    }
                }
                #[cfg(target_os = "linux")]
                unsafe {
                    fwlib::cnc_exitprocess();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            verify_password,
            get_offset_history,
            get_latest_offset_log,
            get_all_machine_states,
            update_tool_settings,
            update_batch_size,
            get_font_size,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
