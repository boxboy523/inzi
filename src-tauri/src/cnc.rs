use std::collections::HashSet;
use std::sync::Arc;
use std::{collections::HashMap, sync::Mutex};

use anyhow::anyhow;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{config::MasterConfig, fwlib::FocasClient, gauge::GaugeResponse};

pub fn spawn_cnc_loop(
    mut receiver: UnboundedReceiver<GaugeResponse>,
    handle_table: Arc<HashMap<u16, FocasClient>>,
    master: Arc<MasterConfig>,
    batch_size: usize,
) {
    let mut gauge_batches = HashMap::new();
    let busy_machines = Arc::new(Mutex::new(HashSet::<u16>::new()));
    tokio::spawn(async move {
        while let Some(gauge_response) = receiver.recv().await {
            let machine_id = gauge_response.machine_id;
            let batch = gauge_batches.entry(machine_id).or_insert_with(Vec::new);
            batch.push(gauge_response);
            if batch.len() >= batch_size {
                let gauges_to_process = batch.drain(..).collect();
                let table_ref = handle_table.clone();
                let master_ref = master.clone();
                let busy_ref = busy_machines.clone();
                {
                    let mut busy_set = busy_ref.lock().unwrap();
                    if busy_set.contains(&machine_id) {
                        println!(
                            "장비 {}번은 아직 이전 보정 작업 중입니다. 이번 배치는 건너뜁니다.",
                            machine_id
                        );
                        continue;
                    }
                    busy_set.insert(machine_id);
                }
                tokio::spawn(async move {
                    if let Err(e) = write_offset(gauges_to_process, &table_ref, &master_ref).await {
                        eprintln!("Failed to write offsets for machine {}: {}", machine_id, e);
                    }
                    let mut busy_set = busy_ref.lock().unwrap();
                    busy_set.remove(&machine_id);
                    println!("장비 {}번 보정 작업 완료 및 플래그 해제", machine_id);
                });
            }
        }
    });
}

async fn write_offset(
    gauges: Vec<GaugeResponse>,
    handle_table: &HashMap<u16, FocasClient>,
    master: &MasterConfig,
) -> anyhow::Result<()> {
    if gauges.is_empty() {
        anyhow::bail!("No gauge data available to write offsets.");
    }
    let machine_id = gauges[0].machine_id;
    let target_tools: [i16; 2] = [11, 12];
    let master_offsets = master
        .offsets
        .get(&machine_id)
        .ok_or_else(|| anyhow!("No master offsets for this machine"))?;
    let handle = handle_table
        .get(&machine_id)
        .ok_or_else(|| anyhow!("No cnc handle for this machine"))?;
    for (i, &tool_num) in target_tools.iter().enumerate() {
        let mut points: Vec<f64> = gauges
            .iter()
            .filter_map(|g| g.points.get(i).copied())
            .collect();
        if points.is_empty() {
            println!(
                "No gauge points available for tool {} on machine {}.",
                tool_num, machine_id
            );
            continue;
        }

        let avg_point = if points.len() > 2 {
            points.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let sum: f64 = points[1..points.len() - 1].iter().sum();
            sum / (points.len() - 2) as f64
        } else {
            let sum: f64 = points.iter().sum();
            sum / points.len() as f64
        };
        let master_offset = match master_offsets.get(&tool_num) {
            Some(&offset) => offset,
            None => {
                println!(
                    "No master offset found for tool {} on machine {}.",
                    tool_num, machine_id
                );
                continue;
            }
        };
        let offset_diff = master_offset - avg_point;
        let cnc_data = (offset_diff * 1000.0).round() as i32;
        match handle.wrtofs(tool_num, 0, cnc_data).await {
            Ok(_) => {
                println!(
                    "Wrote to machine {}: tool {} offset set to {} (raw {})",
                    machine_id, tool_num, avg_point, cnc_data
                );
            }
            Err(e) => {
                println!(
                    "Failed to write to machine {}: tool {}: {}",
                    machine_id, tool_num, e
                );
            }
        }
    }
    Ok(())
}
