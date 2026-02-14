use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::UnboundedSender,
    task,
    time::timeout,
};

pub fn spawn_gauge_stream(
    ip: &str,
    port: u16,
    command_hex: &str,
    channel: UnboundedSender<GaugeResponse>,
) -> anyhow::Result<()> {
    let cmd = hex::decode(command_hex)?;
    let addr = format!("{}:{}", ip, port);
    tokio::spawn(async move {
        loop {
            println!("Attempting to connect to gauge at {}...", addr);
            let mut tcp_stream = match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    println!("Successfully connected to gauge at {}", addr);
                    stream
                }
                Err(e) => {
                    eprintln!("Failed to connect to {}: {}. Retrying in 5s...", addr, e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue; // 다시 연결 시도로 돌아감
                }
            };
            let mut stream_buffer = StreamBuffer::new();
            loop {
                let mut buf = vec![0; 1024];
                if let Err(e) = tcp_stream.write_all(&cmd).await {
                    eprintln!("Write error: {}. Breaking for reconnect...", e);
                    break;
                }
                let n = match timeout(Duration::from_millis(500), tcp_stream.read(&mut buf)).await {
                    Ok(Ok(0)) => {
                        println!("Connection closed by gauge (EOF).");
                        break;
                    }
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => {
                        eprintln!("Read error: {}.", e);
                        break;
                    }
                    Err(_) => continue,
                };
                buf.truncate(n);
                let (new_buffer, response_opt) = stream_buffer.append(buf);
                stream_buffer = new_buffer;
                if let Some(response) = response_opt {
                    println!("Received response: {:?}", response);
                    if let Err(_) = channel.send(response) {
                        eprintln!("Channel receiver dropped. Stopping task.");
                        return;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            }
            println!("Reconnecting to gauge at {} in 5s...", addr);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

#[derive(Debug)]
pub struct GaugeResponse {
    pub machine_id: u16,
    plc_data_on: u16,
    pub points: Vec<f64>,
}

impl GaugeResponse {
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        if bytes.len() < 51 {
            return None;
        }
        let machine_id = u16::from_le_bytes([bytes[11], bytes[12]]);
        let plc_data_on = u16::from_le_bytes([bytes[13], bytes[14]]);
        let points = (0..5)
            .map(|i| {
                let offset = 31 + i * 4;
                let integer = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
                let fractional = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
                integer as f64 + (fractional as f64 / 10000.0)
            })
            .collect();
        Some(Self {
            machine_id,
            plc_data_on,
            points,
        })
    }
}

#[derive(Debug)]
struct StreamBuffer(Vec<u8>);

impl StreamBuffer {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn append(mut self, mut data: Vec<u8>) -> (Self, Option<GaugeResponse>) {
        self.0.append(&mut data);
        drop(data);
        if self.0.len() < 11 {
            return (self, None);
        }
        let length = u16::from_le_bytes([self.0[7], self.0[8]]) as usize;
        if self.0.len() < (length + 9) {
            return (self, None);
        }
        data = self.0.drain(..(length + 9)).collect();
        let response = GaugeResponse::from_bytes(data);
        (self, response)
    }
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
        let handle_result = spawn_gauge_stream("127.0.0.1", port, "500000").await;
        assert!(handle_result.is_ok(), "TCP 연결 또는 스트림 생성 실패");

        let handle = handle_result.unwrap();

        let task_result = handle.await.unwrap();
        assert!(task_result.is_ok(), "루프 실행 중 에러 발생");
    }
}
