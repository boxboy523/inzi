use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::cnc::ToolData;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub gauge: GaugeConfig,
    pub machines: Vec<MachineConfig>,
    pub mapping: MappingConfig,
    pub admin: AdminConfig,
    pub log_path: String,
    pub ui: UiConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiConfig {
    pub font_size: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GaugeConfig {
    pub ip: String,
    pub port: u16,
    pub read_req_hex: String,
    pub write_req_hex_0: String, // D6100=0 (리셋 해제)
    pub write_req_hex: String,   // D6100=1 (리셋 요청)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MachineConfig {
    pub id: u8,       // 패킷에서 오는 식별자 (1, 2)
    pub name: String, // "1호기(OP-10)"
    pub ip: String,   // CNC IP
    pub port: i16,    // Focas 포트 (보통 8193)
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MappingConfig {
    pub tool_data: HashMap<u16, (ToolData, ToolData)>, // machine_id -> (ToolDataUpper, ToolDataLower)
    pub batch_size: HashMap<u16, usize>,               // machine_id -> batch_size
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AdminConfig {
    pub password: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let tool_data = HashMap::from([
            (
                0,
                (
                    ToolData {
                        machine_id: 0,
                        tool_num: 11,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                    ToolData {
                        machine_id: 0,
                        tool_num: 12,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                ),
            ),
            (
                1,
                (
                    ToolData {
                        machine_id: 1,
                        tool_num: 11,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                    ToolData {
                        machine_id: 1,
                        tool_num: 12,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                ),
            ),
            (
                2,
                (
                    ToolData {
                        machine_id: 2,
                        tool_num: 11,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                    ToolData {
                        machine_id: 2,
                        tool_num: 12,
                        basic_size: 48.0,
                        manual_offset: 0.0,
                        offset_rate: 1.0,
                        active: false,
                        avg_gauge: None,
                        final_offset: None,
                    },
                ),
            ),
        ]);
        let batch_size = HashMap::from([(0, 5), (1, 5), (2, 5)]);
        Self {
            gauge: GaugeConfig {
                ip: "127.0.0.1".to_string(),
                port: 5002,
                read_req_hex: "500000FFFF03000C00100001040000701700A81600".to_string(),
                write_req_hex_0: "500000FFFF03000E00200001140000D41700A801000000".to_string(), // D6100=0
                write_req_hex: "500000FFFF03000E00200001140000D41700A801000100".to_string(), // D6100=1
            },
            machines: vec![
                MachineConfig {
                    id: 0,
                    name: "Lathe #1 (OP-10)".to_string(),
                    ip: "dummy".to_string(),
                    port: 8193,
                },
                MachineConfig {
                    id: 1,
                    name: "Lathe #1 (OP-10)".to_string(),
                    ip: "dummy".to_string(),
                    port: 8193,
                },
                MachineConfig {
                    id: 2,
                    name: "Lathe #2 (OP-20)".to_string(),
                    ip: "dummy".to_string(),
                    port: 8193,
                },
            ],
            mapping: MappingConfig {
                tool_data,
                batch_size,
            },
            admin: AdminConfig {
                password: "admin123".to_string(),
            },
            log_path: "logs/log.db".to_string(),
            ui: UiConfig { font_size: 16 },
        }
    }
}

impl AppConfig {
    pub fn load(path: &str) -> Self {
        if let Ok(config_str) = fs::read_to_string(path) {
            serde_json::from_str(&config_str).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let config_json = serde_json::to_string_pretty(self).unwrap();
        fs::write(Path::new(path), config_json)
    }

    pub fn update_from_state(&mut self, state: &AppState) {
        let tool_data = state.tool_data.lock().unwrap();
        let batch_size = state.batch_size.lock().unwrap();
        self.mapping.tool_data = tool_data.clone();
        self.mapping.batch_size = batch_size.clone();
    }
}
