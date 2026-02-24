use std::time::Duration;

use bytes::BytesMut;
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::broadcast::Sender};

use crate::io::{Unzip, IO};
use futures::{stream::SplitStream, SinkExt, StreamExt};
use tokio_util::codec::{Decoder, Encoder, Framed};

pub fn spawn_gauge_stream(
    ip: &str,
    port: u16,
    command_hex: &str,
    channel: Sender<GaugeResponse>,
) -> anyhow::Result<()> {
    if ip == "127.0.0.1" {
        println!("Spawning dummy gauge server for testing...");
        tokio::spawn(async move {
            spawn_dummy_gauge_server(port).await;
        });
    }
    let cmd = hex::decode(command_hex)?;
    let addr = format!("{}:{}", ip, port);
    tokio::spawn(async move {
        loop {
            let tcp_stream = IO::new(match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    println!("Successfully connected to gauge at {}", addr);
                    stream
                }
                Err(e) => {
                    eprintln!("Failed to connect to {}: {}. Retrying in 5s...", addr, e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue; // 다시 연결 시도로 돌아감
                }
            });

            let (sink, stream) = tcp_stream
                .map(|stream| Framed::new(stream, McProtocolCodec).split())
                .unzip();
            let cmd_clone = cmd.clone();
            let channel_clone = channel.clone();
            tokio::select! {
                _ = sink.consume_and_wait(|mut sink| async move {
                    loop {
                        if let Err(e) = sink.send(cmd_clone.clone()).await {
                            eprintln!("Sink send error: {}. Stopping sink task.", e);
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }) => {
                    eprintln!("Sink task ended for {}", addr);
                }
                _ = stream
                    .consume_and_wait(|stream| async move {
                        gauge_get_response(channel_clone, stream).await;
                    }) => {
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
        .fold(channel, |ch, response| async move {
            if let Err(e) = ch.send(response) {
                eprintln!("Channel send error: {}", e);
            }
            ch
        })
        .await;
}

#[derive(Debug, Clone)]
pub struct GaugeResponse {
    pub machine_id: u16,
    pub raw_data: String,
    plc_data_on: u16,
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
            plc_data_on,
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

impl Encoder<Vec<u8>> for McProtocolCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: Vec<u8>, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}

pub async fn spawn_dummy_gauge_server(port: u16) {
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind dummy gauge");
    println!("[Dummy] Fake Gauge Server is running on {}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut machine_id = 1u16;
                    loop {
                        let mut bytes = vec![0u8; 51];
                        bytes[7] = 42; // payload length
                        bytes[8] = 0;

                        let m_bytes = machine_id.to_le_bytes();
                        bytes[11] = m_bytes[0];
                        bytes[12] = m_bytes[1];

                        let ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_millis();
                        let frac = (ms % 100) as i16 - 50;

                        let int_bytes = 48i16.to_le_bytes(); // 96 * 10000 / 2 = 48.0000 기준
                        let frac_bytes = frac.to_le_bytes();

                        bytes[31] = int_bytes[0];
                        bytes[32] = int_bytes[1];
                        bytes[33] = frac_bytes[0];
                        bytes[34] = frac_bytes[1];
                        bytes[35] = int_bytes[0];
                        bytes[36] = int_bytes[1];
                        bytes[37] = frac_bytes[0];
                        bytes[38] = frac_bytes[1];

                        if socket.write_all(&bytes).await.is_err() {
                            break;
                        }
                        println!(
                            "[Dummy] Sent response with machine_id {} and point {}",
                            machine_id,
                            48.0 + (frac as f64) / 10000.0
                        );
                        machine_id = (machine_id + 1) % 3;
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
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
