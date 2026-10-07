use std::time::Duration;

use bytes::BytesMut;
use tokio::{
    net::TcpStream,
    sync::mpsc::{self, UnboundedSender},
};

use futures::{stream::SplitStream, SinkExt, StreamExt};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::{logger::HistoryLogger, HEX_CMDS};

#[derive(Debug, Clone, Copy)]
pub enum GaugeModel {
    G4G700,
    G4G710,
}

impl GaugeModel {
    pub fn from_config(value: &str) -> anyhow::Result<Self> {
        match value {
            "4G700" => Ok(Self::G4G700),
            "4G710" => Ok(Self::G4G710),
            other => anyhow::bail!("Unsupported gauge model: {}", other),
        }
    }

    fn value_registers(self) -> (usize, usize, u16, u16) {
        match self {
            Self::G4G700 => (39, 43, 6014, 6016),
            Self::G4G710 => (47, 51, 6018, 6020),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::G4G700 => "4G700",
            Self::G4G710 => "4G710",
        }
    }
}

#[derive(Debug, Clone)]
pub enum HexCommand {
    Read,
    Write0,
    Write,
}

pub fn spawn_gauge_stream(
    ip: &str,
    port: u16,
    model: &str,
    logger: HistoryLogger,
) -> anyhow::Result<()> {
    let model = GaugeModel::from_config(model)?;
    println!("[Gauge] model={}", model.name());

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
                    println!("[Gauge] connected {}", addr);
                    stream
                }
                Err(e) => {
                    eprintln!("[Gauge ERROR] connect {}: {}. retrying in 5s", addr, e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (mut sink, stream) = Framed::new(tcp_stream, McProtocolCodec { model }).split();
            let logger_clone = logger.clone();
            let (write_tx, mut write_rx) = mpsc::unbounded_channel::<HexCommand>();

            tokio::select! {
                _ = async move {
                    let cmds = HEX_CMDS.get().unwrap();
                    loop {
                        while let Ok(cmd) = write_rx.try_recv() {
                            match cmd {
                                HexCommand::Write => {
                                    #[cfg(debug_assertions)]
                                    println!("[Gauge] ACK D6100=1 -> 0");

                                    if let Err(e) = sink.send(cmds.write_req_hex.as_slice()).await {
                                        eprintln!("[Gauge ERROR] ACK D6100=1 send: {}", e);
                                        return;
                                    }
                                    if let Err(e) = sink.send(cmds.write_req_hex_0.as_slice()).await {
                                        eprintln!("[Gauge ERROR] ACK D6100=0 send: {}", e);
                                        return;
                                    }
                                }
                                HexCommand::Write0 => {
                                    if let Err(e) = sink.send(cmds.write_req_hex_0.as_slice()).await {
                                        eprintln!("[Gauge ERROR] D6100=0 send: {}", e);
                                        return;
                                    }
                                }
                                HexCommand::Read => {}
                            }
                        }

                        if let Err(e) = sink.send(cmds.read_req_hex.as_slice()).await {
                            eprintln!("[Gauge ERROR] read request send: {}", e);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                } => {
                    eprintln!("[Gauge ERROR] sender stopped {}", addr);
                }
                _ = async move {
                    gauge_get_response(logger_clone, stream, write_tx).await;
                } => {
                    eprintln!("[Gauge ERROR] receiver stopped {}", addr);
                }
            }

            println!("[Gauge] disconnected {}. reconnecting in 5s", addr);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

pub async fn gauge_get_response(
    logger: HistoryLogger,
    stream: SplitStream<Framed<TcpStream, McProtocolCodec>>,
    sink: UnboundedSender<HexCommand>,
) {
    stream
        .filter_map(|result| async {
            match result {
                Ok(response) => Some(response),
                Err(e) => {
                    eprintln!("[Gauge ERROR] stream: {}", e);
                    None
                }
            }
        })
        .fold(
            (logger, sink, None::<u16>),
            |(logger, sink, last_state), response| async move {
                #[cfg(debug_assertions)]
                match last_state {
                    Some(previous) if previous != response.measurement_state => {
                        println!(
                            "[Gauge] state {} -> {} line={}",
                            previous, response.measurement_state, response.active_line
                        );
                    }
                    None => {
                        println!(
                            "[Gauge] state={} line={}",
                            response.measurement_state, response.active_line
                        );
                    }
                    _ => {}
                }

                let current_state = response.measurement_state;

                if current_state == PLC_MEASUREMENT_COMPLETE
                    && last_state != Some(PLC_MEASUREMENT_COMPLETE)
                {
                    println!(
                        "[Gauge] COMPLETE line={} value1={:.4} value2={:.4}",
                        response.active_line,
                        response.value1 as f64 / 10000.0,
                        response.value2 as f64 / 10000.0
                    );

                    logger.insert_gauge_response(response);
                    sink.send(HexCommand::Write).unwrap_or_else(|e| {
                        eprintln!("[Gauge ERROR] queue ACK: {}", e);
                    });
                }

                (logger, sink, Some(current_state))
            },
        )
        .await;
}

#[derive(Debug, Clone)]
pub struct GaugeResponse {
    pub active_line: u16,
    pub raw_data: String,
    pub plc_data_on: bool,
    pub measurement_state: u16,
    pub value1: i32,
    pub value2: i32,
}

const PLC_MEASUREMENT_COMPLETE: u16 = 2;
const PLC_RESPONSE_MIN_LEN: usize = 55;

impl GaugeResponse {
    fn from_bytes(bytes: Vec<u8>, model: GaugeModel) -> Option<Self> {
        if bytes.len() < 11 {
            eprintln!("[Gauge ERROR] short MC response: {} bytes", bytes.len());
            return None;
        }

        let end_code = u16::from_le_bytes([bytes[9], bytes[10]]);
        if end_code != 0 {
            eprintln!("[Gauge ERROR] PLC end code: 0x{:04X}", end_code);
            return None;
        }

        if bytes.len() < PLC_RESPONSE_MIN_LEN {
            eprintln!(
                "[Gauge ERROR] short data response: {} bytes, expected at least {}",
                bytes.len(), PLC_RESPONSE_MIN_LEN
            );
            return None;
        }

        let active_line = u16::from_le_bytes([bytes[11], bytes[12]]);
        let measurement_state = u16::from_le_bytes([bytes[13], bytes[14]]);

        let parse_value = |base: usize| -> i32 {
            let integer = i16::from_le_bytes([bytes[base], bytes[base + 1]]);
            let fractional = i16::from_le_bytes([bytes[base + 2], bytes[base + 3]]);
            integer as i32 * 10000 + fractional as i32
        };

        let (value1_base, value2_base, _, _) = model.value_registers();
        let value1 = parse_value(value1_base);
        let value2 = parse_value(value2_base);

        Some(Self {
            active_line,
            raw_data: hex::encode(&bytes),
            plc_data_on: measurement_state == PLC_MEASUREMENT_COMPLETE,
            measurement_state,
            value1,
            value2,
        })
    }
}

pub struct McProtocolCodec {
    model: GaugeModel,
}

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
        Ok(GaugeResponse::from_bytes(data, self.model))
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
                        let mut resp = vec![0u8; PLC_RESPONSE_MIN_LEN];

                        resp[0..7].copy_from_slice(&[0xD0, 0x00, 0x00, 0xFF, 0xFF, 0x03, 0x00]);
                        resp[7..9].copy_from_slice(&[0x2E, 0x00]);
                        resp[9..11].copy_from_slice(&[0x00, 0x00]);

                        toggle_on = if toggle_on == 0 { 2 } else { 0 };

                        resp[11..13].copy_from_slice(&machine_id.to_le_bytes());
                        resp[13..15].copy_from_slice(&toggle_on.to_le_bytes());

                        let ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_millis();
                        let frac = (ms % 100) as i16 - 50;
                        let int_val = 48i16;

                        resp[31..33].copy_from_slice(&int_val.to_le_bytes());
                        resp[33..35].copy_from_slice(&frac.to_le_bytes());
                        resp[35..37].copy_from_slice(&int_val.to_le_bytes());
                        resp[37..39].copy_from_slice(&frac.to_le_bytes());

                        resp[39..41].copy_from_slice(&int_val.to_le_bytes());
                        resp[41..43].copy_from_slice(&frac.to_le_bytes());
                        resp[43..45].copy_from_slice(&int_val.to_le_bytes());
                        resp[45..47].copy_from_slice(&frac.to_le_bytes());

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
            let mut mock_response = vec![0u8; PLC_RESPONSE_MIN_LEN];
            mock_response[7] = 0x2E;
            mock_response[8] = 0;
            mock_response[11] = 1;
            mock_response[12] = 0;
            mock_response[13] = 2;
            mock_response[14] = 0;
            socket.write_all(&mock_response).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        let db_path = std::env::temp_dir().join(format!("inzi-gauge-test-{}.db", port));
        let logger = HistoryLogger::new(db_path.to_str().unwrap());
        // The listener above provides the mock server; avoid spawning another one.
        let handle_result = spawn_gauge_stream("localhost", port, "4G700", logger);
        assert!(handle_result.is_ok(), "TCP 연결 또는 스트림 생성 실패");
    }

    #[test]
    fn test_model_register_layouts() {
        assert_eq!(GaugeModel::G4G700.value_registers(), (39, 43, 6014, 6016));
        assert_eq!(GaugeModel::G4G710.value_registers(), (47, 51, 6018, 6020));
    }
}
