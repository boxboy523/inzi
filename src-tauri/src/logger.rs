use std::path::Path;

use crate::{gauge::GaugeResponse, OffsetLog};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RawGaugeLog {
    pub id: i32,
    pub timestamp: String,
    pub active_line: i32,
    pub tool_type: i32, // 1: 황삭, 2: 정삭
    pub measured_value: f64,
    pub is_used: i32,
}

#[derive(Debug, Clone)]
pub struct HistoryLogger {
    db_path: String,
}

impl HistoryLogger {
    pub fn new(db_path: &str) -> Self {
        let path = db_path.to_string();

        if let Some(parent) = Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).expect("Failed to create log directory");
            }
        }

        let conn = Connection::open(&path).expect("Failed to open database");

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;  
             PRAGMA synchronous = NORMAL;",
        )
        .expect("Failed to set WAL mode");

        conn.execute(
            "CREATE TABLE IF NOT EXISTS offset_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                machine_id INTEGER NOT NULL,
                tool_num INTEGER NOT NULL,
                old_value INTEGER NOT NULL,
                change_amount INTEGER NOT NULL,
                new_value INTEGER NOT NULL,
                success BOOLEAN NOT NULL
            )",
            [],
        )
        .expect("Failed to create offset_history table");

        conn.execute(
            "CREATE TABLE IF NOT EXISTS gauge_raw_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                active_line INTEGER NOT NULL,  -- 1호기, 2호기... (사용자 표시용)
                machine_id INTEGER NOT NULL,   -- 0, 1... (내부 로직용)
                tool_type INTEGER NOT NULL,    -- 1: 황삭(Value1), 2: 정삭(Value2)
                measured_value REAL NOT NULL,
                is_used INTEGER DEFAULT 0      -- 0: 미사용, 1: 사용됨
            )",
            [],
        )
        .expect("Failed to create gauge_raw_logs table");
        Self { db_path: path }
    }

    pub fn log_offset(&self, log: OffsetLog) {
        let path = self.db_path.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = Connection::open(path) {
                let _ = conn.execute(
                   "INSERT INTO offset_history (timestamp, machine_id, tool_num, old_value, change_amount, new_value, success) 
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        log.timestamp.to_rfc3339(),
                        log.machine_id,
                        log.tool_num,
                        log.old_value,
                        log.change_amount,
                        log.new_value,
                        log.success
                    ],
                );
            }
        });
    }

    pub async fn get_offset_history(
        &self,
        machine_id: u16,
        tool_num: i16,
        limit: u32,
    ) -> anyhow::Result<Vec<OffsetLog>> {
        let conn = Connection::open(&self.db_path)?;
        tokio::task::spawn_blocking(move || {
            let mut stmt = conn.prepare(
                "SELECT timestamp, machine_id, tool_num, old_value, change_amount, new_value, success 
                 FROM offset_history 
                 WHERE machine_id = ?1 AND tool_num = ?2 
                 ORDER BY timestamp DESC 
                 LIMIT ?3"
            )?;

            let rows = stmt.query_map(
                params![machine_id, tool_num, limit],
                |row| {
                    Ok(OffsetLog {
                        timestamp: chrono::DateTime::parse_from_rfc3339(
                            row.get::<_, String>(0)?.as_str()
                        ).unwrap().with_timezone(&chrono::Utc),
                        machine_id: row.get(1)?,
                        tool_num: row.get(2)?,
                        old_value: row.get(3)?,
                        change_amount: row.get(4)?,
                        new_value: row.get(5)?,
                        success: row.get(6)?,
                    })
                })?;

            let mut history = Vec::new();
            for log in rows {
                history.push(log?);
            }
            Ok(history)
        })
        .await?
    }

    pub fn get_latest_offset(&self, machine_id: u16, tool_num: i16) -> Option<OffsetLog> {
        let conn = Connection::open(&self.db_path).ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT timestamp, machine_id, tool_num, old_value, change_amount, new_value, success 
                 FROM offset_history 
                 WHERE machine_id = ?1 AND tool_num = ?2 
                 ORDER BY timestamp DESC 
                 LIMIT 1",
            )
            .ok()?;

        let log = stmt
            .query_row(params![machine_id, tool_num], |row| {
                Ok(OffsetLog {
                    timestamp: chrono::DateTime::parse_from_rfc3339(
                        row.get::<_, String>(0)?.as_str(),
                    )
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                    machine_id: row.get(1)?,
                    tool_num: row.get(2)?,
                    old_value: row.get(3)?,
                    change_amount: row.get(4)?,
                    new_value: row.get(5)?,
                    success: row.get(6)?,
                })
            })
            .ok()?;

        Some(log)
    }

    pub fn insert_gauge_response(&self, res: GaugeResponse) {
        let path = self.db_path.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(mut conn) = Connection::open(path) {
                let tx = conn.transaction();
                if let Ok(tx) = tx {
                    // active_line은 1부터 시작하므로, machine_id는 -1 해줌
                    let machine_id = if res.active_line > 0 {
                        res.active_line - 1
                    } else {
                        0
                    };

                    // 1. 황삭 데이터 (Value 1) -> tool_type: 1
                    let _ = tx.execute(
                        "INSERT INTO gauge_raw_logs (active_line, machine_id, tool_type, measured_value, is_used) 
                         VALUES (?1, ?2, 1, ?3, 0)",
                        params![res.active_line, machine_id, res.value1],
                    );

                    // 2. 정삭 데이터 (Value 2) -> tool_type: 2
                    let _ = tx.execute(
                        "INSERT INTO gauge_raw_logs (active_line, machine_id, tool_type, measured_value, is_used) 
                         VALUES (?1, ?2, 2, ?3, 0)",
                        params![res.active_line, machine_id, res.value2],
                    );

                    let _ = tx.commit(); // 둘 다 성공해야 저장
                }
            }
        });
    }

    pub fn fetch_and_process_batch(&self, machine_id: u16, batch_size: usize) -> Option<Vec<i32>> {
        let mut conn = Connection::open(&self.db_path).ok()?;
        let tx = conn.transaction().ok()?;

        {
            // 미사용 데이터 조회 (오래된 순)
            let mut stmt = tx
                .prepare(
                    "SELECT id, measured_value FROM gauge_raw_logs 
                 WHERE machine_id = ?1 AND is_used = 0 
                 ORDER BY timestamp ASC",
                )
                .ok()?;

            let rows = stmt
                .query_map(rusqlite::params![machine_id], |row| {
                    Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)?))
                })
                .ok()?;

            let mut ids = Vec::new();
            let mut values = Vec::new();

            for row in rows.flatten() {
                ids.push(row.0);
                values.push(row.1);
            }

            drop(stmt);

            // 배치 사이즈만큼 데이터가 모였는지 확인
            if values.len() >= batch_size {
                // 앞에서부터 배치 사이즈만큼만 자름
                let target_ids = &ids[0..batch_size];
                let target_values = values[0..batch_size].to_vec();

                let _ = tx.execute(
                    "UPDATE gauge_raw_logs SET is_used = 2 WHERE machine_id = ?1 AND is_used = 1",
                    params![machine_id],
                );
                // 사용 처리 (Update)
                // rusqlite는 배열 바인딩이 복잡하므로 단순 루프로 처리 (배치 사이즈가 작으므로 성능 영향 미미)
                for id in target_ids {
                    let _ = tx.execute(
                        "UPDATE gauge_raw_logs SET is_used = 1 WHERE id = ?1",
                        params![id],
                    );
                }

                let _ = tx.commit();
                return Some(target_values);
            }
        }

        // 배치가 안 찼으면 롤백(자동)되고 None 반환
        None
    }

    pub async fn get_raw_gauge_logs(
        db_path: String,
        machine_id: u16,
        limit: u32,
    ) -> anyhow::Result<Vec<RawGaugeLog>> {
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path)?;
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, active_line, tool_type, measured_value, is_used 
                 FROM gauge_raw_logs 
                 WHERE machine_id = ?1 
                 ORDER BY timestamp DESC LIMIT ?2",
            )?;

            let rows = stmt.query_map(params![machine_id, limit], |row| {
                Ok(RawGaugeLog {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    active_line: row.get(2)?,
                    tool_type: row.get(3)?,
                    measured_value: row.get(4)?,
                    is_used: row.get::<_, i32>(5)?,
                })
            })?;

            let mut result = Vec::new();
            for row in rows.flatten() {
                result.push(row);
            }
            Ok(result)
        })
        .await?
    }
}
