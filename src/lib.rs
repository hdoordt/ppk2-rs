#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

use measurement::{MeasurementAccumulator, MeasurementIterExt, MeasurementMatch};
use serialport::{ClearBuffer::Input, FlowControl, SerialPort};
use std::str::Utf8Error;
use std::sync::mpsc::{self, Receiver, SendError, TryRecvError};
use std::{
    borrow::Cow,
    collections::VecDeque,
    io,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
use thiserror::Error;
use tracing::info;
use types::{DevicePower, LogicPortPins, MeasurementMode, Metadata, SourceVoltage};

use crate::cmd::Command;

pub mod cmd;
pub mod measurement;
pub mod types;

const SPS_MAX: usize = 100_000;

#[derive(Error, Debug)]
/// PPK2 communication or data parsing error.
#[allow(missing_docs)]
pub enum Error {
    #[error("Serial port error: {0}")]
    SerialPort(#[from] serialport::Error),
    #[error("PPK2 not found. Is the device connected and are permissions set correctly?")]
    Ppk2NotFound,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Utf8 error {0}")]
    Utf8(#[from] Utf8Error),
    #[error("Parse error in \"{0}\"")]
    Parse(String),
    #[error("Error sending measurement: {0}")]
    SendMeasurement(#[from] SendError<MeasurementMatch>),
    #[error("Error sending stop signal: {0}")]
    SendStopSignal(#[from] SendError<()>),
    #[error("Worker thread signal error: {0}")]
    WorkerSignalError(#[from] TryRecvError),
    #[error("Error deserializeing a measurement: {0:?}")]
    DeserializeMeasurement(Vec<u8>),
    #[error("Error pausing/resuming measurements")]
    UsageError(String),
}

#[allow(missing_docs)]
pub type Result<T> = std::result::Result<T, Error>;

/// PPK2 device representation.
pub struct Ppk2 {
    port: Box<dyn SerialPort>,
    metadata: Metadata,
}

/// Active measurement session.
pub struct MeasurementSession {
    ppk2: Option<Ppk2>,
    sig_kill: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
    paused: bool,
}

impl MeasurementSession {
    /// Stop measurements and return the owned [Ppk2] instance.
    pub fn stop(mut self) -> Result<Ppk2> {
        self.stop_inner()?;
        Ok(self
            .ppk2
            .take()
            .expect("measurement session should always own Ppk2"))
    }

    /// Pause measurements.
    pub fn pause(&mut self) -> Result<()> {
        if self.paused {
            return Err(Error::UsageError(
                "Can't pause because measurements are already paused".to_owned(),
            ));
        }

        let ppk2 = self.ppk2.as_mut().ok_or_else(|| {
            Error::UsageError("Can't pause because session has already been stopped".to_owned())
        })?;

        ppk2.send_command(Command::AverageStop)?;
        self.paused = true;
        Ok(())
    }

    /// Resume measurements.
    pub fn resume(&mut self) -> Result<()> {
        if !self.paused {
            return Err(Error::UsageError(
                "Can't resume because measurements are already running".to_owned(),
            ));
        }

        let ppk2 = self.ppk2.as_mut().ok_or_else(|| {
            Error::UsageError("Can't resume because session has already been stopped".to_owned())
        })?;

        ppk2.send_command(Command::AverageStart)?;
        self.paused = false;
        Ok(())
    }

    /// Check if this session is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    fn stop_inner(&mut self) -> Result<()> {
        if let Some(sig_kill) = self.sig_kill.take() {
            sig_kill.send(())?;
        }

        if let Some(worker) = self.worker.take() {
            let worker_result = worker
                .join()
                .map_err(|_| Error::UsageError("Worker thread panicked".to_owned()))?;
            worker_result?;
        }

        if let Some(ppk2) = self.ppk2.as_mut() {
            ppk2.send_command(Command::AverageStop)?;
        }

        self.paused = false;
        Ok(())
    }
}

impl Drop for MeasurementSession {
    fn drop(&mut self) {
        if self.ppk2.is_none() {
            return;
        }

        if let Some(sig_kill) = self.sig_kill.take() {
            let _ = sig_kill.send(());
        }

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        if let Some(ppk2) = self.ppk2.as_mut() {
            let _ = ppk2.send_command(Command::AverageStop);
        }
    }
}

impl Ppk2 {
    /// Create a new instance and configure the given [MeasurementMode].
    pub fn new<'a>(path: impl Into<Cow<'a, str>>, mode: MeasurementMode) -> Result<Self> {
        let mut port = serialport::new(path, 9600)
            .timeout(Duration::from_millis(500))
            .flow_control(FlowControl::Hardware)
            .open()?;

        if let Err(e) = port.clear(serialport::ClearBuffer::All) {
            tracing::warn!("failed to clear buffers: {:?}", e);
        }

        // Required to work on Windows.
        if let Err(e) = port.write_data_terminal_ready(true) {
            tracing::warn!("failed to set DTR: {:?}", e);
        }

        let mut ppk2 = Self {
            port,
            metadata: Metadata::default(),
        };

        ppk2.metadata = ppk2.get_metadata()?;
        ppk2.set_power_mode(mode)?;
        Ok(ppk2)
    }

    /// Send a raw command and return the result.
    pub fn send_command(&mut self, command: Command) -> Result<Vec<u8>> {
        self.port.write_all(&Vec::from_iter(command.bytes()))?;
        // Doesn't allocate if expected response length is 0
        let mut response = Vec::with_capacity(command.expected_response_len());
        let mut buf = [0u8; 128];
        while !command.response_complete(&response) {
            let n = self.port.read(&mut buf)?;
            response.extend_from_slice(&buf[..n]);
        }
        Ok(response)
    }

    fn try_get_metadata(&mut self) -> Result<Metadata> {
        let response = self.send_command(Command::GetMetaData)?;
        Metadata::from_bytes(&response)
    }

    /// Get the device metadata.
    pub fn get_metadata(&mut self) -> Result<Metadata> {
        let mut result: Result<Metadata> = Err(Error::Parse("Metadata".to_string()));

        // Retry a few times, as the metadata command sometimes fails
        for _ in 0..3 {
            match self.try_get_metadata() {
                Ok(metadata) => {
                    result = Ok(metadata);
                    break;
                }
                Err(e) => {
                    tracing::warn!("Error fetching metadata: {:?}. Retrying..", e);
                }
            }
        }

        result
    }

    /// Enable or disable the device power.
    pub fn set_device_power(&mut self, power: DevicePower) -> Result<()> {
        self.send_command(Command::DeviceRunningSet(power))?;
        Ok(())
    }

    /// Set the voltage of the device voltage source.
    pub fn set_source_voltage(&mut self, vdd: SourceVoltage) -> Result<()> {
        self.send_command(Command::RegulatorSet(vdd))?;
        Ok(())
    }

    /// Start measurements. Returns a tuple of:
    /// - [Ppk2<Measuring>],
    /// - [Receiver] of [measurement::MeasurementMatch], and
    /// - A closure that can be called to stop the measurement parsing pipeline and return the
    ///   device.
    pub fn start_measurement(
        self,
        sps: usize,
    ) -> Result<(Receiver<MeasurementMatch>, impl FnOnce() -> Result<Self>)> {
        self.start_measurement_matching(LogicPortPins::default(), sps)
    }

    /// Start measurements. Returns a tuple of:
    /// - [Ppk2<Measuring>],
    /// - [Receiver] of [measurement::Result], and
    /// - A closure that can be called to stop the measurement parsing pipeline and return the
    ///   device.
    pub fn start_measurement_matching(
        self,
        pins: LogicPortPins,
        sps: usize,
    ) -> Result<(Receiver<MeasurementMatch>, impl FnOnce() -> Result<Self>)> {
        let (meas_rx, session) = self.start_measurement_matching_session(pins, sps)?;
        let stop = move || session.stop();
        Ok((meas_rx, stop))
    }

    /// Start measurements and return a [MeasurementSession] that supports pause/resume/stop.
    pub fn start_measurement_session(
        self,
        sps: usize,
    ) -> Result<(Receiver<MeasurementMatch>, MeasurementSession)> {
        self.start_measurement_matching_session(LogicPortPins::default(), sps)
    }

    /// Start matching measurements and return a [MeasurementSession] that supports
    /// pause/resume/stop.
    pub fn start_measurement_matching_session(
        mut self,
        pins: LogicPortPins,
        sps: usize,
    ) -> Result<(Receiver<MeasurementMatch>, MeasurementSession)> {
        let (meas_rx, sig_kill, worker) = self.start_measurement_worker(pins, sps)?;

        let session = MeasurementSession {
            ppk2: Some(self),
            sig_kill: Some(sig_kill),
            worker: Some(worker),
            paused: false,
        };

        Ok((meas_rx, session))
    }

    #[allow(clippy::type_complexity)]
    fn start_measurement_worker(
        &mut self,
        pins: LogicPortPins,
        sps: usize,
    ) -> Result<(
        Receiver<MeasurementMatch>,
        mpsc::Sender<()>,
        thread::JoinHandle<Result<()>>,
    )> {
        if sps == 0 || sps > SPS_MAX {
            return Err(Error::UsageError(format!(
                "sps must be in range 1..={SPS_MAX}, got {sps}"
            )));
        }

        // Stuff needed to communicate with the main thread
        // ready allows main thread to signal worker when serial input buf is cleared.
        let ready = Arc::new((Mutex::new(false), Condvar::new()));
        // This channel is for sending measurements to the main thread.
        let (meas_tx, meas_rx) = mpsc::channel::<MeasurementMatch>();
        // This channel allows the main thread to notify that the worker thread can stop
        // parsing data.
        let (sig_tx, sig_rx) = mpsc::channel::<()>();

        let task_ready = ready.clone();
        let mut port = self.port.try_clone()?;
        let metadata = self.metadata.clone();

        let worker = thread::spawn(move || {
            let r = || -> Result<()> {
                // Create an accumulator with the current device metadata
                let mut accumulator = MeasurementAccumulator::new(metadata);
                // First wait for main thread to clear
                // serial port input buffer
                let (lock, cvar) = &*task_ready;
                let _l = cvar
                    .wait_while(lock.lock().unwrap(), |ready| !*ready)
                    .unwrap();

                /* 4 bytes is the size of a single sample, and the PPK pushes 100,000 samples per second.
                   Having size of `buf` at eg.1024 blocks port.read() until the buffer is full with 1024 bytes (128 samples).
                   The measurement returned will be the average of the 128 samples. But we want to get every single sample when
                   requested sps is 100,000. Hence, we set the buffer size to 4 bytes, and read the port in a loop,
                   feeding the accumulator with the data.
                */
                let mut buf = [0u8; 4];
                let mut measurement_buf = VecDeque::with_capacity(SPS_MAX);
                let mut missed = 0;
                loop {
                    // Check whether the main thread has signaled
                    // us to stop
                    match sig_rx.try_recv() {
                        Ok(_) => {
                            info!("Kill signal received from parent thread, quitting measurement matching");
                            return Ok(());
                        }
                        Err(TryRecvError::Empty) => {}
                        Err(e) => return Err(e.into()),
                    }

                    // Now we read chunks and feed them to the accumulator
                    let result = match port.read(&mut buf) {
                        Ok(n) => Ok(n),
                        Err(e) => match e.kind() {
                            io::ErrorKind::TimedOut => {
                                // A timeout is expected when the PPK2 is paused.
                                // We just continue trying to read until we get some data
                                // We can't just increase the read timeout, because then
                                // we will just block and kill signals will be missed
                                continue;
                            }
                            _ => Err(e),
                        },
                    };
                    let n = result?;

                    missed += accumulator.feed_into(&buf[..n], &mut measurement_buf);
                    let len = measurement_buf.len();
                    if len >= SPS_MAX / sps {
                        let measurement = measurement_buf.drain(..).combine_matching(missed, pins);
                        meas_tx.send(measurement)?;
                        missed = 0;
                    }
                }
            };
            let res = r();
            if let Err(e) = &res {
                tracing::error!("Error fetching measurements: {:?}", e);
            };
            res
        });
        self.port.clear(Input)?;

        let (lock, cvar) = &*ready;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cvar.notify_all();

        self.send_command(Command::AverageStart)?;
        Ok((meas_rx, sig_tx, worker))
    }

    /// Reset the device, making the device unusable.
    pub fn reset(mut self) -> Result<()> {
        self.send_command(Command::Reset)?;
        Ok(())
    }

    fn set_power_mode(&mut self, mode: MeasurementMode) -> Result<()> {
        self.send_command(Command::SetPowerMode(mode))?;
        Ok(())
    }
}

/// Try to find the serial port the PPK2 is connected to.
pub fn try_find_ppk2_port() -> Result<String> {
    use serialport::SerialPortType::UsbPort;

    Ok(serialport::available_ports()?
        .into_iter()
        .find(|p| match &p.port_type {
            UsbPort(usb) => usb.vid == 0x1915 && usb.pid == 0xc00a,
            _ => false,
        })
        .ok_or(Error::Ppk2NotFound)?
        .port_name)
}
