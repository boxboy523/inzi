use std::sync::Arc;
use std::{collections::HashMap, sync::Mutex};

use anyhow::anyhow;
use futures::future::join_all;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Receiver;
use tokio_stream::wrappers::BroadcastStream;

use crate::OffsetLog;
use crate::{fwlib::FocasClient, gauge::GaugeResponse, logger::HistoryLogger};

pub struct GaugeBatches {
    batches: HashMap<u16, Vec<i32>>, // (machine_id, tool_num) -> batch of points
    tool_data: Arc<Mutex<HashMap<u16, (ToolData, ToolData)>>>, // machine_id -> (ToolDataUpper , ToolDataLower)
    handle_table: Arc<HashMap<u16, FocasClient>>,
    batch_size: Arc<Mutex<HashMap<u16, usize>>>, // machine_id -> batch_size
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolData {
    pub machine_id: u16,
    pub tool_num: i16,
    pub basic_size: f64,
    pub manual_offset: f64,
    pub offset_rate: f64,
    pub active: bool,
    pub avg_gauge: Option<f64>,
    pub final_offset: Option<f64>,
}

impl ToolData {
    fn get_final_offset(&self) -> Option<f64> {
        if let Some(avg_gauge) = self.avg_gauge {
            let offset_diff = (self.basic_size - avg_gauge + self.manual_offset) * self.offset_rate;
            Some(offset_diff)
        } else {
            None
        }
    }

    fn get_final_offset_as_i32(&self) -> Option<i32> {
        self.get_final_offset()
            .map(|offset| (offset * 10000.0).round() as i32)
    }
}

impl GaugeBatches {
    pub fn new(
        batch_size: Arc<Mutex<HashMap<u16, usize>>>,
        tool_data: Arc<Mutex<HashMap<u16, (ToolData, ToolData)>>>,
        handle_table: Arc<HashMap<u16, FocasClient>>,
    ) -> Self {
        Self {
            batches: HashMap::new(),
            tool_data,
            handle_table,
            batch_size,
        }
    }

    pub fn insert(&mut self, gauge_response: &GaugeResponse) {
        self.batches
            .entry(gauge_response.machine_id)
            .or_insert_with(Vec::new)
            .push(gauge_response.point);
    }

    pub fn extract_all(&mut self) -> anyhow::Result<Vec<(u16, i16, i32)>> {
        let keys = self.handle_table.keys().cloned().collect::<Vec<u16>>();
        keys.into_iter().try_fold(Vec::new(), |mut acc, key| {
            let mut extracted = self.check_and_extract(key)?;
            if let Some(upper) = extracted.0.take() {
                acc.push(upper);
            }
            if let Some(lower) = extracted.1.take() {
                acc.push(lower);
            }
            Ok(acc)
        })
    }

    pub fn check_and_extract(
        &mut self,
        key: u16,
    ) -> anyhow::Result<(Option<(u16, i16, i32)>, Option<(u16, i16, i32)>)> {
        if let Some(handle) = self.handle_table.get(&key) {
            if !handle.is_connected() || handle.is_busy() {
                return Ok((None, None));
            }
        } else {
            self.batches.remove(&key);
            return Err(anyhow!("No CNC client found for machine {}", key));
        }
        let batches = self.batches.remove(&key).unwrap_or_else(Vec::new);
        let batch_size = *self.batch_size.lock().unwrap().get(&key).unwrap_or(&5);
        if batches.len() >= batch_size {
            let avg_point = if batches.len() > 2 {
                let mut sorted = batches.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let sum: f64 = sorted[1..sorted.len() - 1].iter().sum::<i32>() as f64;
                sum / (sorted.len() - 2) as f64
            } else {
                let sum: f64 = batches.iter().sum::<i32>() as f64;
                sum / batches.len() as f64
            };
            let avg_point = avg_point.round() / 10000.0;
            match self.tool_data.lock().unwrap().get_mut(&key) {
                Some((tool_upper, tool_lower)) => {
                    tool_upper.avg_gauge = Some(avg_point);
                    tool_lower.avg_gauge = Some(avg_point);
                    tool_upper.final_offset = tool_upper.get_final_offset();
                    tool_lower.final_offset = tool_lower.get_final_offset();
                    let upper = if tool_upper.active {
                        tool_upper
                            .get_final_offset_as_i32()
                            .map(|offset| (tool_upper.machine_id, tool_upper.tool_num, offset))
                    } else {
                        None
                    };
                    let lower = if tool_lower.active {
                        tool_lower
                            .get_final_offset_as_i32()
                            .map(|offset| (tool_lower.machine_id, tool_lower.tool_num, offset))
                    } else {
                        None
                    };
                    Ok((upper, lower))
                }
                None => Err(anyhow!("No tool data found for machine {}", key)),
            }
        } else {
            self.batches.insert(key, batches);
            Ok((None, None))
        }
    }
}

pub fn spawn_cnc_loop(
    receiver: Receiver<GaugeResponse>,
    handle_table: Arc<HashMap<u16, FocasClient>>,
    tool_data: Arc<Mutex<HashMap<u16, (ToolData, ToolData)>>>,
    batch_size: Arc<Mutex<HashMap<u16, usize>>>,
    logger: Arc<HistoryLogger>,
) -> anyhow::Result<()> {
    let gauge_batches = GaugeBatches::new(batch_size, tool_data, Arc::clone(&handle_table));
    let stream = BroadcastStream::new(receiver);
    tokio::spawn(async move {
        stream
            .fold(
                (gauge_batches, logger),
                |(mut acc, logger), stream_result| async move {
                    let gauge_response = match stream_result {
                        Ok(response) => response,
                        Err(e) => {
                            eprintln!("Stream error: {}", e);
                            return (acc, logger);
                        }
                    };
                    acc.insert(&gauge_response);

                    let results = acc.extract_all().unwrap_or_else(|e| {
                        eprintln!("Batch extraction error: {}", e);
                        Vec::new()
                    });
                    let handle_table_clone = Arc::clone(&acc.handle_table);
                    let logger_clone = Arc::clone(&logger);
                    tokio::spawn(async move {
                        let iter = results.into_iter().map(|(machine_id, tool_num, offset)| {
                            let handle_table = Arc::clone(&handle_table_clone);
                            let logger = Arc::clone(&logger_clone);
                            async move {
                                write_offset_to_cnc(
                                    handle_table,
                                    logger,
                                    machine_id,
                                    tool_num,
                                    offset,
                                )
                                .await
                            }
                        });
                        join_all(iter).await.into_iter().for_each(|res| {
                            if let Err(e) = res {
                                eprintln!("Error writing offset to CNC: {}", e);
                            }
                        });
                    });
                    (acc, logger)
                },
            )
            .await;
    });
    Ok(())
}

pub async fn update_offset_logs(
    logger: Arc<HistoryLogger>,
    handle_table: Arc<HashMap<u16, FocasClient>>,
    tool_data: Arc<Mutex<HashMap<u16, (ToolData, ToolData)>>>,
) {
    let mut last_offsets: HashMap<(u16, i16), i32> = HashMap::new();
    loop {
        tool_data
            .lock()
            .unwrap()
            .iter()
            .for_each(|(&machine_id, (tool_upper, tool_lower))| {
                if let Some(client) = handle_table.get(&machine_id) {
                    if !client.is_connected() || client.is_busy() {
                        return;
                    }
                    if let Ok(current_upper) = client.rdtofs(tool_upper.tool_num, 0) {
                        let current_upper_value = current_upper.data as i32;
                        let last_upper_value = last_offsets
                            .get(&(machine_id, tool_upper.tool_num))
                            .cloned()
                            .unwrap_or(current_upper_value);
                        if current_upper_value != last_upper_value {
                            println!(
                                "Offset change detected for machine {}, tool {}: {} -> {}",
                                machine_id,
                                tool_upper.tool_num,
                                last_upper_value,
                                current_upper_value
                            );
                            logger.log(OffsetLog {
                                timestamp: chrono::Utc::now(),
                                machine_id,
                                tool_num: tool_upper.tool_num,
                                old_value: last_upper_value,
                                change_amount: current_upper_value - last_upper_value,
                                new_value: current_upper_value,
                                success: true,
                            });
                        }
                        last_offsets.insert((machine_id, tool_upper.tool_num), current_upper_value);
                    }
                    if let Ok(current_lower) = client.rdtofs(tool_lower.tool_num, 0) {
                        let current_lower_value = current_lower.data as i32;
                        let last_lower_value = last_offsets
                            .get(&(machine_id, tool_lower.tool_num))
                            .cloned()
                            .unwrap_or(current_lower_value);
                        if current_lower_value != last_lower_value {
                            println!(
                                "Offset change detected for machine {}, tool {}: {} -> {}",
                                machine_id,
                                tool_lower.tool_num,
                                last_lower_value,
                                current_lower_value
                            );
                            logger.log(OffsetLog {
                                timestamp: chrono::Utc::now(),
                                machine_id,
                                tool_num: tool_lower.tool_num,
                                old_value: last_lower_value,
                                change_amount: current_lower_value - last_lower_value,
                                new_value: current_lower_value,
                                success: true,
                            });
                        }
                        last_offsets.insert((machine_id, tool_lower.tool_num), current_lower_value);
                    }
                }
            });
        tokio::time::sleep(tokio::time::Duration::from_millis(5000)).await;
    }
}

async fn write_offset_to_cnc(
    handle_table: Arc<HashMap<u16, FocasClient>>,
    logger: Arc<HistoryLogger>,
    machine_id: u16,
    tool_num: i16,
    offset_diff: i32,
) -> anyhow::Result<()> {
    if let Some(client) = handle_table.get(&machine_id) {
        let current_offset = client.rdtofs(tool_num, 0)?;
        let client_clone = client.clone();
        let old_offset = current_offset.data as i32;
        let new_offset = current_offset.data as i32 + offset_diff;
        let result = client_clone.wrtofs(tool_num, 0, new_offset).await;

        logger.log(OffsetLog {
            timestamp: chrono::Utc::now(),
            machine_id,
            tool_num,
            old_value: old_offset,
            change_amount: offset_diff,
            new_value: new_offset,
            success: result.is_ok(),
        });
        Ok(())
    } else {
        Err(anyhow!("No CNC client found for machine {}", machine_id))
    }
}
