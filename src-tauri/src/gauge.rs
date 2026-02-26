use std::time::Duration;

use bytes::BytesMut;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{
        broadcast::Sender,
        mpsc::{self, UnboundedSender},
    },
};

use futures::{stream::SplitStream, SinkExt, StreamExt};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::HEX_CMDS;

#[derive(Debug, Clone)]
pub enum HexCommand {
    Read,
    Write0,
    Write1,
}

pub fn spawn_gauge_stream(
    ip: &str,
    port: u16,
    channel: Sender<GaugeResponse>,
) -> anyhow::Result<()> {
    if ip == "127.0.0.1" {
        println!("Spawning dummy gauge server for testing...");
        tokio::spawn(async move {
            spawn_dummy_gauge_server(port).await;
        });
    }
    let addr = format!("{}:{}", ip, port);
    tokio::spawn(async move {
        loop {
            let tcp_stream = match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    println!("Successfully connected to gauge at {}", addr);
                    stream
                }
                Err(e) => {
                    eprintln!("Failed to connect to {}: {}. Retrying in 5s...", addr, e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (mut sink, stream) = Framed::new(tcp_stream, McProtocolCodec).split();
            let channel_clone = channel.clone();
            let (sink_tx1, mut sink_rx) = mpsc::unbounded_channel::<HexCommand>();
            let sink_tx2 = sink_tx1.clone();

            tokio::spawn(async move {
                while let Some(cmd) = sink_rx.recv().await {
                    let cmds = HEX_CMDS.get().unwrap();
                    let hex_cmd = match cmd {
                        HexCommand::Read => &cmds.read_req_hex,
                        HexCommand::Write0 => &cmds.write_req_hex_0,
                        HexCommand::Write1 => &cmds.write_req_hex_1,
                    };
                    if let Err(e) = sink.send(hex_cmd).await {
                        eprintln!(
                            "Failed to send command to gauge: {}. Stopping sink task.",
                            e
                        );
                        break;
                    }
                }
            });

            tokio::select! {
                _ = async move {
                    loop {
                        if let Err(e) = sink_tx1.send(HexCommand::Read) {
                            eprintln!("Sink send error: {}. Stopping sink task.", e);
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                } => {
                    eprintln!("Sink task ended for {}", addr);
                }
                _ = async move {
                        gauge_get_response(channel_clone, stream, sink_tx2).await;
                    } => {
                        eprintln!("Stream task ended for {}", addr);
                    }
            }

            println!(
                "Disconnected from gauge at {}. Attempting to reconnect...",
                addr
            );
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

pub async fn gauge_get_response(
    channel: Sender<GaugeResponse>,
    stream: SplitStream<Framed<TcpStream, McProtocolCodec>>,
    sink: UnboundedSender<HexCommand>,
) {
    stream
        .filter_map(|result| async {
            match result {
                Ok(response) => Some(response),
                Err(e) => {
                    eprintln!("Stream error: {}", e);
                    None
                }
            }
        })
        .fold(
            (channel, sink, false),
            |(ch, sink, mut last_plc_on), response| async move {
                if response.plc_data_on && !last_plc_on {
                    println!(
                        "Received new gauge data for machine {}: point = {}, raw = {}",
                        response.machine_id, response.point, response.raw_data
                    );
                    if let Err(e) = ch.send(response.clone()) {
                        eprintln!("Failed to send gauge response to channel: {}", e);
                    }
                    sink.send(HexCommand::Write1).unwrap_or_else(|e| {
                        eprintln!("Failed to send write command: {}", e);
                    });
                } else if !response.plc_data_on && last_plc_on {
                    sink.send(HexCommand::Write0).unwrap_or_else(|e| {
                        eprintln!("Failed to send write command: {}", e);
                    });
                }
                last_plc_on = response.plc_data_on;
                (ch, sink, last_plc_on)
            },
        )
        .await;
}

#[derive(Debug, Clone)]
pub struct GaugeResponse {
    pub machine_id: u16,
    pub raw_data: String,
    plc_data_on: bool,
    pub point: i32,
}

impl GaugeResponse {
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        if bytes.len() < 51 {
            return None;
        }
        let machine_id = u16::from_le_bytes([bytes[11], bytes[12]]);
        let plc_data_on = u16::from_le_bytes([bytes[13], bytes[14]]);
        let point = (0..2)
            .map(|i| {
                let offset = 31 + i * 4;
                let integer = i16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
                let fractional = i16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
                integer as i32 * 10000 + fractional as i32
            })
            .sum::<i32>()
            / 2;
        Some(Self {
            machine_id,
            raw_data: hex::encode(&bytes),
            plc_data_on: plc_data_on == 1,
            point,
        })
    }
}

pub struct McProtocolCodec;

impl Decoder for McProtocolCodec {
    type Item = GaugeResponse;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 11 {
            return Ok(None);
        }
        let length = u16::from_le_bytes([src[7], src[8]]) as usize;
        if src.len() < (length + 9) {
            return Ok(None);
        }
        let data = src.split_to(length + 9).to_vec();
        Ok(GaugeResponse::from_bytes(data))
    }
}

impl Encoder<&[u8]> for McProtocolCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: &[u8], dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        dst.extend_from_slice(item);
        Ok(())
    }
}

pub async fn spawn_dummy_gauge_server(port: u16) {
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind dummy gauge");
    println!("[Dummy] Fake Binary Gauge Server is running on {}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut machine_id = 1u16;
                    let mut toggle_on = 0u16; // ▼ 추가: 0과 1을 번갈아가며 보낼 변수

                    loop {
                        use tokio::io::AsyncWriteExt;
                        let mut resp = vec![0u8; 61];

                        // (1~3번 헤더 부분은 동일)
                        resp[0..7].copy_from_slice(&[0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00]);
                        resp[7..9].copy_from_slice(&[0x34, 0x00]);
                        resp[9..11].copy_from_slice(&[0x00, 0x00]);

                        // ▼ 매 주기마다 신호를 0 -> 1 -> 0 으로 토글
                        toggle_on = if toggle_on == 0 { 1 } else { 0 };

                        // D6000: Machine ID
                        resp[11..13].copy_from_slice(&machine_id.to_le_bytes());

                        // D6001: PlcDataOn (이제 무조건 1이 아니라 toggle_on 값이 들어감)
                        resp[13..15].copy_from_slice(&toggle_on.to_le_bytes());

                        // 가짜 데이터 생성 부분 (동일)
                        let ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_millis();
                        let frac = (ms % 100) as i16 - 50;
                        let int_val = 48i16;

                        resp[31..33].copy_from_slice(&frac.to_le_bytes());
                        resp[33..35].copy_from_slice(&int_val.to_le_bytes());
                        resp[35..37].copy_from_slice(&frac.to_le_bytes());
                        resp[37..39].copy_from_slice(&int_val.to_le_bytes());

                        if socket.write_all(&resp).await.is_err() {
                            break;
                        }

                        println!(
                            "[Dummy] Sent Binary (PlcDataOn: {}) for machine {}",
                            toggle_on, machine_id
                        );

                        // ▼ 신호가 0으로 떨어질 때 기계 번호를 바꿔줍니다.
                        if toggle_on == 0 {
                            machine_id = if machine_id == 1 { 2 } else { 1 };
                        }

                        // PC가 0.2초마다 폴링하므로, 더미 서버는 0.5초나 1초마다 상태를 바꿉니다.
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
                    dbg!()
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_gauge_tcp_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            let mut mock_response = vec![0u8; 51];
            mock_response[7] = 42;
            mock_response[8] = 0;
            mock_response[11] = 1;
            mock_response[12] = 0;
            mock_response[31] = 10;
            mock_response[32] = 0;
            socket.write_all(&mock_response).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        let handle_result = spawn_gauge_stream("127.0.0.1", port, "500000", tx);
        assert!(handle_result.is_ok(), "TCP 연결 또는 스트림 생성 실패");
    }
}
