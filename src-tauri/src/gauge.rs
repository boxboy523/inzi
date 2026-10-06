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
