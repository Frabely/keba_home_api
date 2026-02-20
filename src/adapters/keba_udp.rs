use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

const UDP_TIMEOUT_SECONDS: u64 = 2;
const UDP_BUFFER_SIZE: usize = 4096;

pub trait KebaClient: Send + Sync + 'static {
    fn get_report2(&self) -> Result<Value, KebaClientError>;
    fn get_report3(&self) -> Result<Value, KebaClientError>;
}

#[derive(Debug, Error)]
pub enum KebaClientError {
    #[error("failed to resolve KEBA endpoint: {0}")]
    Resolve(std::io::Error),
    #[error("udp communication failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse KEBA response as JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct KebaUdpClient {
    target: std::net::SocketAddr,
    timeout: Duration,
}

impl KebaUdpClient {
    pub fn new(host: &str, port: u16) -> Result<Self, KebaClientError> {
        let mut addrs = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(KebaClientError::Resolve)?;
        let target = addrs.next().ok_or_else(|| {
            KebaClientError::Resolve(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no socket address resolved for KEBA endpoint",
            ))
        })?;

        Ok(Self {
            target,
            timeout: Duration::from_secs(UDP_TIMEOUT_SECONDS),
        })
    }

    fn send_command(&self, command: &str) -> Result<Value, KebaClientError> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_read_timeout(Some(self.timeout))?;
        socket.set_write_timeout(Some(self.timeout))?;
        socket.send_to(command.as_bytes(), self.target)?;

        let mut buffer = [0_u8; UDP_BUFFER_SIZE];
        let (size, _) = socket.recv_from(&mut buffer)?;
        serde_json::from_slice(&buffer[..size]).map_err(KebaClientError::from)
    }
}

impl KebaClient for KebaUdpClient {
    fn get_report2(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 2")
    }

    fn get_report3(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 3")
    }
}
