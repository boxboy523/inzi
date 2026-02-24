use crate::OffsetLog;
use rusqlite::{params, Connection, Result as SqlResult};
use tokio::sync::mpsc; // 위에서 정의한 구조체

pub struct HistoryLogger {
    sender: mpsc::UnboundedSender<OffsetLog>,
}

impl HistoryLogger {
    pub fn new(db_path: &str) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<OffsetLog>();
        let path = db_path.to_string();

        std::thread::spawn(move || {
            if let Some(parent_dir) = std::path::Path::new(&path).parent() {
                if !parent_dir.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent_dir).expect("Failed to create log directory");
                }
            }

            let conn = Connection::open(path).expect("Failed to open database");

            conn.execute(
                "CREATE TABLE IF NOT EXISTS offset_history (
                    id INTEGER PRIMARY KEY,
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
            .expect("Failed to create table");

            while let Some(log) = rx.blocking_recv() {
                conn.execute(
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
                ).unwrap_or_else(|e| {eprintln!("Failed to insert log: {}", e); 0});
            }
        });

        Self { sender: tx }
    }

    pub async fn get_latest_log(
        db_path: String,
        machine_id: u16,
        tool_num: i16,
    ) -> anyhow::Result<Option<OffsetLog>> {
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path)?;
            let mut stmt = conn.prepare(
                "SELECT timestamp, machine_id, tool_num, old_value, change_amount, new_value, success 
                 FROM offset_history 
                 WHERE machine_id = ?1 AND tool_num = ?2 
                 ORDER BY timestamp DESC 
                 LIMIT 1"
            )?;

            let mut rows = stmt.query(params![machine_id, tool_num])?;
            if let Some(row) = rows.next()? {
                Ok(Some(Self::row_to_log(row)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    pub async fn get_history(
        db_path: String,
        machine_id: u16,
        tool_num: i16,
        limit: u32,
    ) -> anyhow::Result<Vec<OffsetLog>> {
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path)?;
            let mut stmt = conn.prepare(
                "SELECT timestamp, machine_id, tool_num, old_value, change_amount, new_value, success 
                 FROM offset_history 
                 WHERE machine_id = ?1 AND tool_num = ?2 
                 ORDER BY timestamp DESC 
                 LIMIT ?3"
            )?;

            let rows = stmt.query_map(params![machine_id, tool_num, limit], |row| {
                Self::row_to_log(row)
            })?;

            let mut history = Vec::new();
            for log in rows {
                history.push(log?);
            }
            Ok(history)
        })
        .await?
    }

    fn row_to_log(row: &rusqlite::Row) -> SqlResult<OffsetLog> {
        let timestamp_str: String = row.get(0)?;
        Ok(OffsetLog {
            timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                .unwrap()
                .with_timezone(&chrono::Utc),
            machine_id: row.get(1)?,
            tool_num: row.get(2)?,
            old_value: row.get(3)?,
            change_amount: row.get(4)?,
            new_value: row.get(5)?,
            success: row.get(6)?,
        })
    }

    pub fn log(&self, log: OffsetLog) {
        let _ = self.sender.send(log);
    }
}
