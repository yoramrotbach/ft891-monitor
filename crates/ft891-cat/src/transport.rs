//! Links to the radio: a real serial port or the built-in simulator. Both
//! refuse any command the allow-list doesn't accept, so no caller can send
//! something unsafe by mistake.

use crate::cat;
use crate::sim::SimRadio;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub trait CatPort: Send {
    /// Send a read command and return the answer, including its `;`.
    fn query(&mut self, cmd: &str) -> io::Result<String>;
    /// Send a set command (the radio doesn't answer these).
    fn send(&mut self, cmd: &str) -> io::Result<()>;
}

fn refuse(cmd: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("command {cmd:?} is not on the allow-list"))
}

fn rejected(cmd: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("the radio rejected {cmd}"))
}

// ---------------------------------------------------------------- serial

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopBits {
    One,
    #[default]
    Two,
}

pub struct SerialCat {
    port: Box<dyn serialport::SerialPort>,
    timeout: Duration,
}

impl SerialCat {
    /// Open the radio's CAT port. `rts_cts`: hardware flow control, which the
    /// FT-891 expects on its USB "Enhanced" port while menu 05-08 CAT RTS is
    /// ENABLE (the default).
    pub fn open(name: &str, baud: u32, stop: StopBits, rts_cts: bool) -> io::Result<Self> {
        let mut port = serialport::new(name, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(match stop {
                StopBits::One => serialport::StopBits::One,
                StopBits::Two => serialport::StopBits::Two,
            })
            .flow_control(if rts_cts { serialport::FlowControl::Hardware } else { serialport::FlowControl::None })
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(|e| io::Error::other(format!("{name}: {e}")))?;
        // DTR is never needed for CAT. On the radio's *Standard* USB port and
        // on many interfaces DTR keys CW or PTT, so keep it low. RTS is left to
        // the flow-control setting; without flow control it is dropped too.
        let _ = port.write_data_terminal_ready(false);
        if !rts_cts {
            let _ = port.write_request_to_send(false);
        }
        let _ = port.clear(serialport::ClearBuffer::All);
        Ok(Self { port, timeout: Duration::from_millis(300) })
    }

    fn write(&mut self, cmd: &str) -> io::Result<()> {
        self.port.write_all(cmd.as_bytes())?;
        self.port.flush()
    }
}

impl CatPort for SerialCat {
    fn query(&mut self, cmd: &str) -> io::Result<String> {
        if !cat::is_allowed(cmd) {
            return Err(refuse(cmd));
        }
        let _ = self.port.clear(serialport::ClearBuffer::Input);
        self.write(cmd)?;
        let mut out = Vec::with_capacity(32);
        let mut buf = [0u8; 64];
        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            match self.port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if let Some(end) = out.iter().position(|&b| b == b';') {
                        out.truncate(end + 1);
                        let s = String::from_utf8_lossy(&out).trim().to_string();
                        if s.ends_with("?;") {
                            return Err(rejected(cmd));
                        }
                        return Ok(s);
                    }
                    if out.len() > 128 {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, format!("no answer to {cmd}")))
    }

    fn send(&mut self, cmd: &str) -> io::Result<()> {
        if !cat::is_allowed(cmd) {
            return Err(refuse(cmd));
        }
        self.write(cmd)?;
        // Give the radio time to act before the next poll (Hamlib uses 50 ms).
        std::thread::sleep(Duration::from_millis(50));
        Ok(())
    }
}

// ---------------------------------------------------------------- demo

pub struct DemoCat {
    pub radio: Arc<Mutex<SimRadio>>,
    delay: Duration,
}

impl DemoCat {
    pub fn new() -> Self {
        Self::with_radio(Arc::new(Mutex::new(SimRadio::new())))
    }
    pub fn with_radio(radio: Arc<Mutex<SimRadio>>) -> Self {
        // About the wire time of a short command and answer at 38400 baud.
        Self { radio, delay: Duration::from_millis(4) }
    }
    pub fn instant(mut self) -> Self {
        self.delay = Duration::ZERO;
        self
    }
}

impl Default for DemoCat {
    fn default() -> Self {
        Self::new()
    }
}

impl CatPort for DemoCat {
    fn query(&mut self, cmd: &str) -> io::Result<String> {
        if !cat::is_allowed(cmd) {
            return Err(refuse(cmd));
        }
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let r = self.radio.lock().unwrap().respond(cmd);
        match r.as_str() {
            "" => Err(io::Error::new(io::ErrorKind::TimedOut, format!("no answer to {cmd}"))),
            "?;" => Err(rejected(cmd)),
            _ => Ok(r),
        }
    }

    fn send(&mut self, cmd: &str) -> io::Result<()> {
        if !cat::is_allowed(cmd) {
            return Err(refuse(cmd));
        }
        match self.radio.lock().unwrap().respond(cmd).as_str() {
            "?;" => Err(rejected(cmd)),
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------- ports

#[derive(serde::Serialize)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

pub fn list_ports() -> Vec<PortInfo> {
    let mut v: Vec<PortInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let description = match p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    let mut d = u.product.unwrap_or_default();
                    if let Some(m) = u.manufacturer {
                        if !d.contains(&m) {
                            d = format!("{m} {d}");
                        }
                    }
                    d.trim().to_string()
                }
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                serialport::SerialPortType::PciPort => "PCI".into(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            PortInfo { name: p.port_name, description }
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_link_refuses_unlisted_commands() {
        let radio = Arc::new(Mutex::new(SimRadio::new()));
        let mut p = DemoCat::with_radio(radio.clone()).instant();
        assert_eq!(p.send("TX1;").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(p.query("AC002;").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(p.query("FA;").is_ok());
        // The refused commands never reached the radio.
        let log = &radio.lock().unwrap().log;
        assert_eq!(log, &vec!["FA;".to_string()]);
    }
}
