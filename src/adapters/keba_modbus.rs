use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::adapters::keba_udp::{KebaClient, KebaClientError};

const MODBUS_TIMEOUT_SECONDS: u64 = 2;

// KEBA register map (Modbus TCP):
// 1000: State (u32)
// 1036: Total energy
// 1502: Energy present session
const REG_STATE: u16 = 1000;
const REG_TOTAL_ENERGY: u16 = 1036;
const REG_PRESENT_ENERGY: u16 = 1502;

#[derive(Debug)]
pub struct KebaModbusClient {
    target: std::net::SocketAddr,
    unit_id: u8,
    energy_factor_wh: f64,
    transaction_id: AtomicU16,
}

impl KebaModbusClient {
    pub fn new(
        host: &str,
        port: u16,
        unit_id: u8,
        energy_factor_wh: f64,
    ) -> Result<Self, KebaClientError> {
        let mut addrs = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(KebaClientError::Resolve)?;
        let target = addrs.next().ok_or_else(|| {
            KebaClientError::Resolve(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no socket address resolved for KEBA modbus endpoint",
            ))
        })?;

        Ok(Self {
            target,
            unit_id,
            energy_factor_wh,
            transaction_id: AtomicU16::new(1),
        })
    }

    fn read_input_u32(&self, address: u16) -> Result<u32, KebaClientError> {
        let transaction_id = self.transaction_id.fetch_add(1, Ordering::Relaxed);

        let mut stream = TcpStream::connect_timeout(&self.target, Duration::from_secs(2))
            .map_err(KebaClientError::Io)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(MODBUS_TIMEOUT_SECONDS)))
            .map_err(KebaClientError::Io)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(MODBUS_TIMEOUT_SECONDS)))
            .map_err(KebaClientError::Io)?;

        // MBAP(7) + PDU(5): read input registers (0x04), quantity=2
        let request = [
            (transaction_id >> 8) as u8,
            transaction_id as u8,
            0x00,
            0x00,
            0x00,
            0x06,
            self.unit_id,
            0x04,
            (address >> 8) as u8,
            address as u8,
            0x00,
            0x02,
        ];
        stream.write_all(&request).map_err(KebaClientError::Io)?;

        let mut header = [0_u8; 7];
        stream
            .read_exact(&mut header)
            .map_err(KebaClientError::Io)?;
        let response_len = u16::from_be_bytes([header[4], header[5]]) as usize;
        if response_len < 3 {
            return Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "modbus response too short",
            )));
        }

        let mut pdu = vec![0_u8; response_len - 1];
        stream.read_exact(&mut pdu).map_err(KebaClientError::Io)?;

        if pdu[0] != 0x04 {
            return Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected modbus function code: {}", pdu[0]),
            )));
        }
        if pdu.len() < 6 || pdu[1] != 4 {
            return Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "modbus payload has unexpected byte count",
            )));
        }

        Ok(u32::from_be_bytes([pdu[2], pdu[3], pdu[4], pdu[5]]))
    }
}

impl KebaClient for KebaModbusClient {
    fn get_report2(&self) -> Result<Value, KebaClientError> {
        let state = self.read_input_u32(REG_STATE)?;
        let plugged = u8::from(state >= 2);
        Ok(serde_json::json!({
            "Plug": plugged,
            "State": state
        }))
    }

    fn get_report3(&self) -> Result<Value, KebaClientError> {
        let present_raw = self.read_input_u32(REG_PRESENT_ENERGY)?;
        let total_raw = self.read_input_u32(REG_TOTAL_ENERGY)?;

        let present_kwh = (present_raw as f64) * self.energy_factor_wh / 1000.0;
        let total_kwh = (total_raw as f64) * self.energy_factor_wh / 1000.0;

        Ok(serde_json::json!({
            "Energy (present session)": present_kwh,
            "Energy (total)": total_kwh
        }))
    }
}
