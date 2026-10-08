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
                timestamp DATETIME DEFAULT (datetime('now', 'localtime')),
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
        if res.active_line == 0 || res.value1 == 0 || res.value2 == 0 {
            #[cfg(debug_assertions)]
            println!(
                "[Gauge DB] DROP line={} value1={} value2={}",
                res.active_line, res.value1, res.value2
            );
            return;
        }

        let path = self.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let result = (|| -> rusqlite::Result<()> {
                let mut conn = Connection::open(path)?;
                let tx = conn.transaction()?;
                // Measurement lines start at 1; internal machine IDs start at 0.
                let machine_id = res.active_line - 1;
                for (tool_type, value) in [(1, res.value1), (2, res.value2)] {
                    tx.execute(
                        "INSERT INTO gauge_raw_logs (active_line, machine_id, tool_type, measured_value, is_used)
                         VALUES (?1, ?2, ?3, ?4, 0)",
                        params![res.active_line, machine_id, tool_type, value],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    #[cfg(debug_assertions)]
                    println!(
                        "[Gauge DB] INSERT line={} value1={} value2={}",
                        res.active_line, res.value1, res.value2
                    );
                }
                Err(e) => {
                    eprintln!("[Gauge DB] INSERT ERROR: line={}: {}", res.active_line, e);
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
                 WHERE machine_id = ?1 AND active_line > 0 AND is_used = 0
                 ORDER BY id ASC",
                )
                .ok()?;

            let rows = stmt
                .query_map(rusqlite::params![machine_id], |row| {
                    let value = row.get::<_, f64>(1)?.round() as i32;
                    Ok((row.get::<_, i32>(0)?, value))
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
                 ORDER BY id DESC LIMIT ?2",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDb {
        logger: HistoryLogger,
        directory: std::path::PathBuf,
    }

    impl TestDb {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "inzi-logger-test-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            let logger = HistoryLogger::new(directory.join("logs.db").to_str().unwrap());
            Self { logger, directory }
        }

        fn insert(&self, active_line: u16, value1: i32, value2: i32) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                self.logger.insert_gauge_response(GaugeResponse {
                    active_line,
                    value1,
                    value2,
                    raw_data: String::new(),
                    plc_data_on: true,
                    measurement_state: 2,
                });
            });
            // Runtime shutdown waits for queued blocking writes to finish.
            drop(runtime);
        }

        fn row_count(&self) -> i64 {
            Connection::open(&self.logger.db_path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM gauge_raw_logs", [], |row| row.get(0))
                .unwrap()
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn master_measurement_is_not_saved_as_machine_one() {
        let db = TestDb::new();
        // Nonzero values isolate the line-zero regression from value filtering.
        db.insert(0, 480000, 480010);
        assert_eq!(db.row_count(), 0);
        assert_eq!(db.logger.fetch_and_process_batch(0, 2), None);
    }

    #[test]
    fn zero_in_either_value_drops_the_whole_measurement() {
        let db = TestDb::new();
        for (value1, value2) in [(0, 480000), (480000, 0), (0, 0)] {
            db.insert(1, value1, value2);
            assert_eq!(db.row_count(), 0);
        }
    }

    #[test]
    fn valid_measurements_keep_both_values_and_machine_mapping() {
        let db = TestDb::new();
        for line in 1..=3 {
            db.insert(line, 480000, 480010);
            assert_eq!(
                db.logger.fetch_and_process_batch(line - 1, 2),
                Some(vec![480000, 480010])
            );
            assert_eq!(db.logger.fetch_and_process_batch(line - 1, 2), None);
        }
        assert_eq!(db.row_count(), 6);
    }

    #[test]
    fn historical_master_rows_do_not_fill_a_machine_batch() {
        let db = TestDb::new();
        let conn = Connection::open(&db.logger.db_path).unwrap();
        for tool_type in [1, 2] {
            conn.execute(
                "INSERT INTO gauge_raw_logs (active_line, machine_id, tool_type, measured_value)
                 VALUES (0, 0, ?1, 0)",
                params![tool_type],
            )
            .unwrap();
        }
        assert_eq!(db.logger.fetch_and_process_batch(0, 2), None);
        db.insert(1, 480000, 480010);
        assert_eq!(db.logger.fetch_and_process_batch(0, 4), None);
        assert_eq!(
            db.logger.fetch_and_process_batch(0, 2),
            Some(vec![480000, 480010])
        );
    }
}
