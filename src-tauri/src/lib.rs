use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::{collections::HashMap, sync::Mutex};

use chrono::Local;
use serde::Serialize;
use tauri::{Manager, State};

use crate::config::AdminConfig;
use crate::{
    cnc::spawn_cnc_loop, config::AppConfig, fwlib::FocasClient, gauge::spawn_gauge_stream,
};

pub mod cnc;
pub mod config;
pub mod fwlib;
pub mod gauge;

pub struct AppState {
    pub handle_table: Arc<HashMap<u16, FocasClient>>,
    pub config: Mutex<AppConfig>,
}

#[derive(Debug, Serialize)]
pub struct ToolData {
    pub tool_num: i16,
    pub name: String,
    pub offset: f64,
    life_count: i16,
}

#[derive(Serialize)]
pub struct MachineStatus {
    pub id: u8,
    pub name: String,
    pub ip: String,
    pub port: i16,
    pub connected: bool,
    pub tools: Vec<ToolData>,
}

#[derive(Serialize)]
pub struct GaugeData {
    ip: String,
    is_connected: bool,
    last_hex: String,
    raw_data: String,
    master_offset: f64,
    current_val: f64,
}

#[tauri::command]
async fn get_machine_status(state: State<'_, AppState>) -> Result<Vec<MachineStatus>, String> {
    let config = state.config.lock().map_err(|_| "Config Mutex poisoned")?;

    let mut statuses = Vec::new();

    for machine in &config.machines {
        let connected = if let Some(client) = state.handle_table.get(&(machine.id as u16)) {
            client.is_connected()
        } else {
            false
        };

        statuses.push(MachineStatus {
            id: machine.id,
            name: machine.name.clone(),
            ip: machine.ip.clone(),
            port: machine.port,
            connected,
        });
    }

    Ok(statuses)
}

#[tauri::command]
async fn read_tool_offset(
    machine_id: u16,
    tool_num: i16,
    state: State<'_, AppState>,
) -> Result<f64, String> {
    let handle = state
        .handle_table
        .get(&machine_id)
        .ok_or_else(|| "해당 장비를 찾을 수 없습니다.".to_string())?;

    // fwlib.rs에 구현된 rdtofs 호출
    let res = handle.rdtofs(tool_num, 0).map_err(|e| e.to_string())?;

    // raw 데이터를 mm 단위로 변환 (0.001 기준)
    Ok(res.data as f64 / 1000.0)
}

#[tauri::command]
fn verify_password(input: String, state: State<AdminConfig>) -> bool {
    input == state.password
}

#[tauri::command]
fn log_offset_change(
    machine_id: u16,
    tool_num: i16,
    old_val: f64,
    new_val: f64,
) -> Result<(), String> {
    let now = Local::now();

    if let Err(e) = fs::create_dir_all("log") {
        return Err(format!("로그 폴더 생성 실패: {}", e));
    }

    let file_name = format!("log/{}.txt", now.format("%Y-%m-%d"));

    let log_msg = format!(
        "[{}] Machine: #{} | Tool: T{} | Offset Changed: {:.3} -> {:.3}\n",
        now.format("%H:%M:%S"),
        machine_id,
        tool_num,
        old_val,
        new_val
    );

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_name)
        .map_err(|e| format!("로그 파일 열기 실패 ({}): {}", file_name, e))?;

    file.write_all(log_msg.as_bytes())
        .map_err(|e| format!("로그 쓰기 실패: {}", e))?;

    println!("로그 저장됨: {} >> {}", file_name, log_msg.trim());
    Ok(())
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
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let handle_table = Arc::new(handle_table);
            let master_config = Arc::new(config.master.clone());
            app.manage(AppState {
                handle_table: handle_table.clone(),
                config: Mutex::new(config.clone()),
            });
            let batch_size = config.gauge.gauge_batch_size;
            tauri::async_runtime::spawn(async move {
                spawn_cnc_loop(rx, handle_table, master_config, batch_size);
            });

            tauri::async_runtime::spawn(async move {
                spawn_gauge_stream(
                    &config.gauge.ip,
                    config.gauge.port,
                    &config.gauge.command_hex,
                    tx,
                );
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let app_handle = window.app_handle();
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Ok(config_guard) = state.config.lock() {
                        println!("Exiting application, Save config: {:?}", *config_guard);
                        if let Err(e) = config_guard.save("config.json") {
                            println!("Failed to save config: {}", e);
                        } else {
                            println!("Config saved successfully.");
                        }
                    }
                }
                #[cfg(target_os = "linux")]
                unsafe {
                    fwlib::cnc_exitprocess();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_machine_status,
            read_tool_offset
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
