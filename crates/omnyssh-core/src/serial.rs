//! Local serial ports (COM ports on Windows) for the serial terminal and the
//! receive-only serial monitor.
//!
//! Each open port is owned by one std thread that alternates between draining
//! control messages and a short-timeout read, so no async serial backend is
//! needed. Received bytes are handed to a caller-supplied callback, which keeps
//! this module free of frontend types.

use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// How long one read or one write attempt waits before the loop checks for
/// new input again. Because it bounds writes too, a single stalled write
/// (CTS low, XOFF, a device not reading) cannot block past this — the loop
/// instead keeps unsent bytes in a pending buffer and retries next iteration.
const IO_TIMEOUT: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

/// Line settings for one port, e.g. `COM3` at 115200 8N1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    pub port: String,
    pub baud_rate: u32,
    /// 5 to 8.
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow_control: FlowControl,
}

/// A port present on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// What to open, e.g. `COM3`.
    pub name: String,
    /// What the driver calls the device, e.g. `USB-SERIAL CH340 (COM3)`.
    pub description: Option<String>,
}

/// What a session hands its callback.
#[derive(Debug, PartialEq, Eq)]
pub enum SerialOutput {
    /// Bytes received from the device.
    Data(Vec<u8>),
    /// The session ended; sent exactly once. `None` when the caller closed it,
    /// the reason when the port failed (adapter unplugged, driver error).
    Closed(Option<String>),
}

enum Ctrl {
    Write(Vec<u8>),
    Close,
}

/// Handle to an open port. Dropping it closes the port and waits for its
/// thread to exit, so the port handle is released before `drop` returns —
/// Windows opens COM ports exclusively, and a close-then-reopen must not race
/// the OS releasing it.
pub struct SerialSession {
    ctrl: Sender<Ctrl>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SerialSession {
    /// Opens `config.port` and starts its session thread. Fails right away when
    /// the port is missing or held by another program, so the caller can report
    /// it instead of opening a dead tab.
    ///
    /// `on_output` runs on the session thread, including while `Drop` joins it,
    /// so it must not take a lock the dropping caller may hold.
    pub fn open(
        config: &SerialConfig,
        on_output: impl FnMut(SerialOutput) + Send + 'static,
    ) -> Result<Self> {
        let port = serialport::new(&config.port, config.baud_rate)
            .data_bits(data_bits(config.data_bits)?)
            .parity(match config.parity {
                Parity::None => serialport::Parity::None,
                Parity::Odd => serialport::Parity::Odd,
                Parity::Even => serialport::Parity::Even,
            })
            .stop_bits(match config.stop_bits {
                StopBits::One => serialport::StopBits::One,
                StopBits::Two => serialport::StopBits::Two,
            })
            .flow_control(match config.flow_control {
                FlowControl::None => serialport::FlowControl::None,
                FlowControl::Software => serialport::FlowControl::Software,
                FlowControl::Hardware => serialport::FlowControl::Hardware,
            })
            .timeout(IO_TIMEOUT)
            .open()
            .with_context(|| format!("cannot open {}", config.port))?;
        Ok(Self::spawn(port, on_output))
    }

    /// Runs a session over any byte stream; `open` uses it for a real port, the
    /// tests for a fake one.
    ///
    /// `on_output` runs on the session thread, including while `Drop` joins it,
    /// so it must not take a lock the dropping caller may hold.
    pub fn spawn<P>(port: P, on_output: impl FnMut(SerialOutput) + Send + 'static) -> Self
    where
        P: Read + Write + Send + 'static,
    {
        let (ctrl, rx) = mpsc::channel();
        let thread = thread::spawn(move || run(port, rx, on_output));
        Self {
            ctrl,
            thread: Some(thread),
        }
    }

    /// Queues bytes for the device. A no-op once the session has ended.
    pub fn write(&self, data: &[u8]) {
        let _ = self.ctrl.send(Ctrl::Write(data.to_vec()));
    }

    /// Asks the session to close the port. Non-blocking: the thread finishes
    /// asynchronously. `Drop` is what waits for it.
    pub fn close(&self) {
        let _ = self.ctrl.send(Ctrl::Close);
    }
}

impl Drop for SerialSession {
    fn drop(&mut self) {
        self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The ports present right now, with `COMn` names ordered numerically
/// (`COM2` before `COM10`).
pub fn list_ports() -> Result<Vec<PortInfo>> {
    let mut ports: Vec<PortInfo> = serialport::available_ports()
        .context("cannot list serial ports")?
        .into_iter()
        .map(|p| PortInfo {
            description: describe(&p.port_type),
            name: p.port_name,
        })
        .collect();
    sort_ports(&mut ports);
    Ok(ports)
}

fn describe(kind: &serialport::SerialPortType) -> Option<String> {
    match kind {
        serialport::SerialPortType::UsbPort(usb) => Some(
            usb.product
                .clone()
                .unwrap_or_else(|| format!("USB {:04X}:{:04X}", usb.vid, usb.pid)),
        ),
        serialport::SerialPortType::BluetoothPort => Some("Bluetooth".to_string()),
        _ => None,
    }
}

/// Orders `COMn` names numerically by sorting on length before name, so
/// shorter numbers (fewer digits) sort first.
fn sort_ports(ports: &mut [PortInfo]) {
    ports.sort_by(|a, b| (a.name.len(), &a.name).cmp(&(b.name.len(), &b.name)));
}

fn data_bits(bits: u8) -> Result<serialport::DataBits> {
    Ok(match bits {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        8 => serialport::DataBits::Eight,
        other => bail!("unsupported data bits: {other}"),
    })
}

/// The session thread: drain queued input into a pending buffer and attempt
/// one write of it, then — unless closing — wait up to `IO_TIMEOUT` for bytes.
/// Runs until closed or the port fails; reports `Closed` exactly once.
///
/// Writes never call `write_all`/`flush`: on a stalled line those can block for
/// as long as the OS lets them (on Windows, `FlushFileBuffers` can wait
/// forever), holding the port's exclusive lock. Instead each iteration makes
/// one bounded `write` attempt and keeps whatever did not fit in `pending` for
/// the next iteration.
fn run<P: Read + Write>(
    mut port: P,
    ctrl: Receiver<Ctrl>,
    mut on_output: impl FnMut(SerialOutput),
) {
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let mut closing = false;
        loop {
            match ctrl.try_recv() {
                Ok(Ctrl::Write(data)) => pending.extend_from_slice(&data),
                Ok(Ctrl::Close) | Err(TryRecvError::Disconnected) => {
                    closing = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        if !pending.is_empty() {
            match port.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                    ) => {}
                Err(e) => {
                    on_output(SerialOutput::Closed(Some(e.to_string())));
                    return;
                }
            }
        }

        if closing {
            // Best effort: keep writing while each attempt makes progress
            // (each is bounded by IO_TIMEOUT); anything else just stops so
            // closing is never blocked on a still-stalled line.
            while !pending.is_empty() {
                match port.write(&pending) {
                    Ok(0) => break,
                    Ok(n) => pending.drain(..n),
                    Err(_) => break,
                };
            }
            on_output(SerialOutput::Closed(None));
            return;
        }

        match port.read(&mut buf) {
            // Windows never returns this; POSIX does on hang-up.
            Ok(0) => {
                on_output(SerialOutput::Closed(Some("port closed".to_string())));
                return;
            }
            Ok(n) => on_output(SerialOutput::Data(buf[..n].to_vec())),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                ) => {}
            Err(e) => {
                on_output(SerialOutput::Closed(Some(e.to_string())));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// A port that returns scripted reads, then times out forever; and scripted
    /// write results, then accepts everything. Records what was actually
    /// written.
    struct FakePort {
        reads: VecDeque<io::Result<Vec<u8>>>,
        writes: VecDeque<io::Result<usize>>,
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl FakePort {
        fn new(reads: Vec<io::Result<Vec<u8>>>) -> (Self, Arc<Mutex<Vec<u8>>>) {
            Self::with_writes(reads, Vec::new())
        }

        /// Like `new`, but scripts the result of the first `writes.len()` calls
        /// to `write` (e.g. stalls before the port accepts data); calls beyond
        /// that accept the whole buffer.
        fn with_writes(
            reads: Vec<io::Result<Vec<u8>>>,
            writes: Vec<io::Result<usize>>,
        ) -> (Self, Arc<Mutex<Vec<u8>>>) {
            let written = Arc::new(Mutex::new(Vec::new()));
            let port = Self {
                reads: reads.into(),
                writes: writes.into(),
                written: Arc::clone(&written),
            };
            (port, written)
        }
    }

    impl Read for FakePort {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.reads.pop_front() {
                Some(Ok(chunk)) => {
                    buf[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                Some(Err(e)) => Err(e),
                None => {
                    thread::sleep(Duration::from_millis(1));
                    Err(io::Error::new(ErrorKind::TimedOut, "timed out"))
                }
            }
        }
    }

    impl Write for FakePort {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.writes.pop_front() {
                Some(Ok(n)) => {
                    self.written.lock().unwrap().extend_from_slice(&buf[..n]);
                    Ok(n)
                }
                Some(Err(e)) => Err(e),
                None => {
                    self.written.lock().unwrap().extend_from_slice(buf);
                    Ok(buf.len())
                }
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Spawns a session on `port`, returning it and a receiver of its outputs.
    fn start(port: FakePort) -> (SerialSession, mpsc::Receiver<SerialOutput>) {
        let (tx, rx) = mpsc::channel();
        let session = SerialSession::spawn(port, move |out| {
            let _ = tx.send(out);
        });
        (session, rx)
    }

    fn next(rx: &mpsc::Receiver<SerialOutput>) -> SerialOutput {
        rx.recv_timeout(Duration::from_secs(2))
            .expect("session produced no output")
    }

    #[test]
    fn forwards_received_bytes() {
        let (port, _) = FakePort::new(vec![Ok(b"hello\n".to_vec())]);
        let (_session, rx) = start(port);
        assert_eq!(next(&rx), SerialOutput::Data(b"hello\n".to_vec()));
    }

    #[test]
    fn read_timeouts_are_not_errors() {
        let timeout = io::Error::new(ErrorKind::TimedOut, "timed out");
        let (port, _) = FakePort::new(vec![Err(timeout), Ok(b"x".to_vec())]);
        let (_session, rx) = start(port);
        assert_eq!(next(&rx), SerialOutput::Data(b"x".to_vec()));
    }

    #[test]
    fn writes_input_to_the_port() {
        let (port, written) = FakePort::new(vec![]);
        let (session, rx) = start(port);
        session.write(b"AT\r\n");
        session.close();
        // Close is handled after the queued write, so once it is reported the
        // bytes are on the port.
        assert_eq!(next(&rx), SerialOutput::Closed(None));
        assert_eq!(written.lock().unwrap().as_slice(), b"AT\r\n");
    }

    #[test]
    fn close_reports_a_clean_end() {
        let (port, _) = FakePort::new(vec![]);
        let (session, rx) = start(port);
        session.close();
        assert_eq!(next(&rx), SerialOutput::Closed(None));
    }

    #[test]
    fn dropping_the_handle_ends_the_session() {
        let (port, _) = FakePort::new(vec![]);
        let (session, rx) = start(port);
        drop(session);
        assert_eq!(next(&rx), SerialOutput::Closed(None));
    }

    #[test]
    fn a_read_error_ends_the_session_with_its_reason() {
        let unplugged = io::Error::new(ErrorKind::BrokenPipe, "device removed");
        let (port, _) = FakePort::new(vec![Err(unplugged)]);
        let (_session, rx) = start(port);
        assert_eq!(
            next(&rx),
            SerialOutput::Closed(Some("device removed".to_string()))
        );
    }

    #[test]
    fn a_stalled_write_does_not_end_the_session() {
        let stall = io::Error::new(ErrorKind::TimedOut, "timed out");
        let (port, written) = FakePort::with_writes(vec![], vec![Ok(0), Err(stall), Ok(0)]);
        let (session, rx) = start(port);
        session.write(b"AT\r\n");

        // A few stalled write attempts happen in the background; the session
        // must stay alive rather than treating the stall as an error.
        thread::sleep(Duration::from_millis(50));
        assert!(
            rx.try_recv().is_err(),
            "session closed while the write was stalled"
        );

        // Once the port accepts data again, the bytes land, in order.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if written.lock().unwrap().as_slice() == b"AT\r\n" {
                break;
            }
            assert!(Instant::now() < deadline, "write never completed");
            thread::sleep(Duration::from_millis(5));
        }

        session.close();
        assert_eq!(next(&rx), SerialOutput::Closed(None));
    }

    #[test]
    fn a_write_error_ends_the_session_with_its_reason() {
        let unplugged = io::Error::new(ErrorKind::BrokenPipe, "device removed");
        let (port, _) = FakePort::with_writes(vec![], vec![Err(unplugged)]);
        let (session, rx) = start(port);
        session.write(b"AT\r\n");
        assert_eq!(
            next(&rx),
            SerialOutput::Closed(Some("device removed".to_string()))
        );
    }

    #[test]
    fn an_empty_read_reports_the_port_closed() {
        let (port, _) = FakePort::new(vec![Ok(vec![])]);
        let (_session, rx) = start(port);
        assert_eq!(
            next(&rx),
            SerialOutput::Closed(Some("port closed".to_string()))
        );
    }

    #[test]
    fn only_five_to_eight_data_bits_are_accepted() {
        assert!(data_bits(8).is_ok());
        assert!(data_bits(5).is_ok());
        assert!(data_bits(4).is_err());
        assert!(data_bits(9).is_err());
    }

    #[test]
    fn opening_a_missing_port_fails() {
        let config = SerialConfig {
            port: "OMNYSSH_NO_SUCH_PORT".to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
        };
        let err = SerialSession::open(&config, |_| {})
            .err()
            .expect("must fail");
        assert!(format!("{err:#}").contains("OMNYSSH_NO_SUCH_PORT"));
    }

    #[test]
    fn ports_sort_in_numeric_order() {
        let mut ports: Vec<PortInfo> = ["COM10", "COM3", "COM1"]
            .iter()
            .map(|n| PortInfo {
                name: n.to_string(),
                description: None,
            })
            .collect();
        sort_ports(&mut ports);
        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["COM1", "COM3", "COM10"]);
    }
}
