use std::time::Duration;

use bytes::BytesMut;
use tokio::{
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
    Write0, // D6100=0 (리셋 해제)
    Write,  // D6100=1 (리셋 요청)
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
            let (write_tx, mut write_rx) = mpsc::unbounded_channel::<HexCommand>();

            tokio::select! {
                _ = async move {
                    let cmds = HEX_CMDS.get().unwrap();
                    loop {
                        // Write 요청이 있으면 우선 처리
                        while let Ok(cmd) = write_rx.try_recv() {
                            match cmd {
                                HexCommand::Write => {
                                    // D6100=1 전송
                                    if let Err(e) = sink.send(cmds.write_req_hex.as_slice()).await {
                                        eprintln!("Write1 send error: {}. Stopping sink task.", e);
                                        return;
                                    }
                                    // D6100=0 즉시 전송 (리셋 해제)
                                    if let Err(e) = sink.send(cmds.write_req_hex_0.as_slice()).await {
                                        eprintln!("Write0 send error: {}. Stopping sink task.", e);
                                        return;
                                    }
                                }
                                HexCommand::Write0 => {
                                    if let Err(e) = sink.send(cmds.write_req_hex_0.as_slice()).await {
                                        eprintln!("Write0 send error: {}. Stopping sink task.", e);
                                        return;
                                    }
                                }
                                _ => {}
                            }
                        }
                        // Read 요청 송신
                        if let Err(e) = sink.send(cmds.read_req_hex.as_slice()).await {
                            eprintln!("Read send error: {}. Stopping sink task.", e);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                } => {
                    eprintln!("Sink task ended for {}", addr);
                }
                _ = async move {
                    gauge_get_response(channel_clone, stream, write_tx).await;
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
                        "Measurement complete for line {}: raw = {}",
                        response.active_line, response.raw_data
                    );
                    if let Err(e) = ch.send(response.clone()) {
                        eprintln!("Failed to send gauge response to channel: {}", e);
                    }
                    // D6100=1: 측정 데이터 리셋 요청 (폴링루프가 Write0을 자동으로 처리)
                    sink.send(HexCommand::Write).unwrap_or_else(|e| {
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
pub struct LineMeasurement {
    pub line_id: u16,
    pub value1: i32,
    pub value2: i32,
}

#[derive(Debug, Clone)]
pub struct GaugeResponse {
    pub active_line: u16,
    pub raw_data: String,
    pub plc_data_on: bool,
    pub lines: [LineMeasurement; 3],
}

const PLC_MEASUREMENT_COMPLETE: u16 = 2;
const PLC_RESPONSE_MIN_LEN: usize = 55; // 9 header + 2 end_code + 44 data (D6000~D6021)

impl GaugeResponse {
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        if bytes.len() < 11 {
            return None;
        }

        let end_code = u16::from_le_bytes([bytes[9], bytes[10]]);
        if end_code != 0 {
            eprintln!("PLC Error Code Received: {:04X}", end_code);
            return None;
        }

        // D6021까지 필요: bytes[11 + 21*2 + 1] = bytes[54]
        if bytes.len() < PLC_RESPONSE_MIN_LEN {
            return None;
        }

        let active_line = u16::from_le_bytes([bytes[11], bytes[12]]); // D6000
        let plc_data_on_raw = u16::from_le_bytes([bytes[13], bytes[14]]); // D6001

        // D6010 = bytes[11 + 10*2] = bytes[31]
        // 2워드(4바이트)당 1측정값: 정수부(2바이트) + 소수부(2바이트)
        let parse_value = |base: usize| -> i32 {
            let integer = i16::from_le_bytes([bytes[base], bytes[base + 1]]);
            let fractional = i16::from_le_bytes([bytes[base + 2], bytes[base + 3]]);
            integer as i32 * 10000 + fractional as i32
        };

        // 라인1: D6010-11(bytes31-34), D6012-13(bytes35-38)
        // 라인2: D6014-15(bytes39-42), D6016-17(bytes43-46)
        // 라인3: D6018-19(bytes47-50), D6020-21(bytes51-54)
        let lines = [
            LineMeasurement {
                line_id: 1,
                value1: parse_value(31),
                value2: parse_value(35),
            },
            LineMeasurement {
                line_id: 2,
                value1: parse_value(39),
                value2: parse_value(43),
            },
            LineMeasurement {
                line_id: 3,
                value1: parse_value(47),
                value2: parse_value(51),
            },
        ];

        Some(Self {
            active_line,
            raw_data: hex::encode(&bytes),
            plc_data_on: plc_data_on_raw == PLC_MEASUREMENT_COMPLETE,
            lines,
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
                    let mut toggle_on = 0u16;

                    loop {
                        use tokio::io::AsyncWriteExt;
                        // 55 bytes: 9 header + 2 end_code + 44 data (22 words)
                        let mut resp = vec![0u8; PLC_RESPONSE_MIN_LEN];

                        resp[0..7].copy_from_slice(&[0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00]);
                        // length = 2 (end_code) + 44 (22 words) = 46 = 0x2E
                        resp[7..9].copy_from_slice(&[0x2E, 0x00]);
                        resp[9..11].copy_from_slice(&[0x00, 0x00]);

                        toggle_on = if toggle_on == 0 { 2 } else { 0 };

                        // D6000: active_line (machine_id 1~3)
                        resp[11..13].copy_from_slice(&machine_id.to_le_bytes());

                        // D6001: PlcDataOn (toggle: 2=측정완료, 0=알수없음)
                        resp[13..15].copy_from_slice(&toggle_on.to_le_bytes());

                        // 가짜 측정 데이터
                        let ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_millis();
                        let frac = (ms % 100) as i16 - 50;
                        let int_val = 48i16;

                        // 라인1: D6010-D6013 (bytes[31..39])
                        resp[31..33].copy_from_slice(&int_val.to_le_bytes());
                        resp[33..35].copy_from_slice(&frac.to_le_bytes());
                        resp[35..37].copy_from_slice(&int_val.to_le_bytes());
                        resp[37..39].copy_from_slice(&frac.to_le_bytes());
                        // 라인2: D6014-D6017 (bytes[39..47])
                        resp[39..41].copy_from_slice(&int_val.to_le_bytes());
                        resp[41..43].copy_from_slice(&frac.to_le_bytes());
                        resp[43..45].copy_from_slice(&int_val.to_le_bytes());
                        resp[45..47].copy_from_slice(&frac.to_le_bytes());
                        // 라인3: D6018-D6021 (bytes[47..55])
                        resp[47..49].copy_from_slice(&int_val.to_le_bytes());
                        resp[49..51].copy_from_slice(&frac.to_le_bytes());
                        resp[51..53].copy_from_slice(&int_val.to_le_bytes());
                        resp[53..55].copy_from_slice(&frac.to_le_bytes());

                        if socket.write_all(&resp).await.is_err() {
                            break;
                        }

                        println!(
                            "[Dummy] Sent Binary (PlcDataOn: {}) for line {}",
                            toggle_on, machine_id
                        );

                        if toggle_on == 0 {
                            machine_id = if machine_id >= 3 { 1 } else { machine_id + 1 };
                        }

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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_gauge_tcp_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            // 55 bytes: 9 header + 2 end_code + 44 data (22 words D6000~D6021)
            let mut mock_response = vec![0u8; PLC_RESPONSE_MIN_LEN];
            // length field = 55 - 9 = 46 = 0x2E
            mock_response[7] = 0x2E;
            mock_response[8] = 0;
            // active_line = 1
            mock_response[11] = 1;
            mock_response[12] = 0;
            // plc_data_on = 2 (측정완료)
            mock_response[13] = 2;
            mock_response[14] = 0;
            // line1 value1 integer part
            mock_response[31] = 10;
            mock_response[32] = 0;
            socket.write_all(&mock_response).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        let (tx, _) = tokio::sync::broadcast::channel(100);
        let handle_result = spawn_gauge_stream("127.0.0.1", port, tx);
        assert!(handle_result.is_ok(), "TCP 연결 또는 스트림 생성 실패");
    }
}
