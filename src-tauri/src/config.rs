use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub gauge: GaugeConfig,
    pub machines: Vec<MachineConfig>,
    pub master: MasterConfig,
    pub admin: AdminConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GaugeConfig {
    pub ip: String,
    pub port: u16,
    pub command_hex: String, // "500000FFFF..." 같은 명령어
    pub gauge_batch_size: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MachineConfig {
    pub id: u8,       // 패킷에서 오는 식별자 (1, 2)
    pub name: String, // "1호기(OP-10)"
    pub ip: String,   // CNC IP
    pub port: i16,    // Focas 포트 (보통 8193)
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MasterConfig {
    pub offsets: HashMap<u16, HashMap<i16, f64>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AdminConfig {
    pub password: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let offsets = HashMap::from([
            (1, HashMap::from([(11, 48.0), (12, 48.0)])),
            (2, HashMap::from([(11, 48.0), (12, 48.0)])),
        ]);
        Self {
            gauge: GaugeConfig {
                ip: "192.168.0.100".to_string(),
                port: 5002,
                command_hex: "500000FFFF03000E00200001140000D41700A801000000".to_string(),
                gauge_batch_size: 5,
            },
            machines: vec![
                MachineConfig {
                    id: 1,
                    name: "Lathe #1 (OP-10)".to_string(),
                    ip: "192.168.0.145".to_string(),
                    port: 8193,
                },
                MachineConfig {
                    id: 2,
                    name: "Lathe #2 (OP-20)".to_string(),
                    ip: "192.168.0.146".to_string(),
                    port: 8193,
                },
            ],
            master: MasterConfig { offsets },
            admin: AdminConfig {
                password: "admin123".to_string(),
            },
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
}
