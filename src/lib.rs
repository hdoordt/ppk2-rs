#![doc = include_str!("../README.md")]

use crossbeam::channel::{Receiver, SendError, Sender, TryRecvError};
use measurement::MeasurementAccumulator;
use serialport::{ClearBuffer::Input, FlowControl, SerialPort};
use state::{Idle, Measuring, State};
use std::{
    borrow::Cow,
    collections::VecDeque,
    io,
    marker::PhantomData,
    string::FromUtf8Error,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
use thiserror::Error;
use types::{DevicePower, Metadata, PowerMode, SourceVoltage};

use crate::cmd::Command;

pub mod cmd;
pub mod measurement;
pub mod types;

pub mod state {
    //! Device state definitions, used for typestate setup.

    /// A state that indicates the power mode was set.
    pub trait Ready: State {}
    /// General device state. Cannot be implemented by users.
    pub trait State: Sealed {}

    macro_rules! state {
        ($state:ident, $doc:literal) => {
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $state;
            impl Sealed for $state {}
            impl State for $state {}
        };
    }

    use self::sealed::Sealed;
    mod sealed {
        pub trait Sealed {}
    }

    state!(Idle, "Device is idle");
    state!(Measuring, "Device is measuring");
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Serial port error: {0}")]
    SerialPort(#[from] serialport::Error),
    #[error("PPK2 not found. Is the device connected and are permissions set correctly?")]
    Ppk2NotFound,
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Utf8 error {0}")]
    Utf8(#[from] FromUtf8Error),
    #[error("Parse error in \"{0}\"")]
    Parse(String),
    #[error("Error sending measurement: {0}")]
    SendMeasurement(#[from] SendError<measurement::Result>),
    #[error("Worker thread signal error: {0}")]
    WorkerSignalError(#[from] TryRecvError),
    #[error("Error deserializeing a measurement: {0:?}")]
    DeserializeMeasurement(Vec<u8>),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Ppk2<S: State> {
    port: Box<dyn SerialPort>,
    metadata: Metadata,
    _state: PhantomData<S>,
}

impl<S: State> Ppk2<S> {
    pub fn reset(mut self) -> Result<()> {
        self.send_command(Command::Reset)?;
        Ok(())
    }

    fn into_state<T: State>(self) -> Ppk2<T> {
        Ppk2 {
            metadata: self.metadata,
            port: self.port,
            _state: PhantomData,
        }
    }

    fn send_command(&mut self, command: Command) -> Result<Vec<u8>> {
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
}

impl Ppk2<Idle> {
    pub fn new<'a>(path: impl Into<Cow<'a, str>>, mode: PowerMode) -> Result<Self> {
        let port = serialport::new(path, 9600)
            .timeout(Duration::from_millis(500))
            .flow_control(FlowControl::Hardware)
            .open()?;
        let mut ppk2 = Self {
            port,
            metadata: Metadata::default(),
            _state: PhantomData,
        };

        ppk2.metadata = ppk2.get_metadata()?;
        ppk2.set_power_mode(mode)?;
        Ok(ppk2)
    }

    pub fn get_metadata(&mut self) -> Result<Metadata> {
        let response = self.send_command(Command::GetMetaData)?;
        Metadata::parse(response)
    }

    pub fn set_device_power(&mut self, power: DevicePower) -> Result<()> {
        self.send_command(Command::DeviceRunningSet(power))?;
        Ok(())
    }

    pub fn set_source_voltage(&mut self, vdd: SourceVoltage) -> Result<()> {
        self.send_command(Command::RegulatorSet(vdd))?;
        Ok(())
    }

    pub fn start_measuring(
        mut self,
    ) -> Result<(
        Ppk2<Measuring>,
        Receiver<measurement::Result>,
        impl FnOnce() -> std::result::Result<(), SendError<()>>,
    )> {
        // Stuff needed to communicate with the main thread
        // ready allows main thread to signal worker when serial input buf is cleared.
        let ready = Arc::new((Mutex::new(false), Condvar::new()));
        // This channel is for sending measurements to the main thread.
        let (meas_tx, meas_rx) = crossbeam::channel::bounded::<measurement::Result>(1024);
        // This channel allows the main thread to notify that the worker thread can stop
        // parsing data.
        let (sig_tx, sig_rx) = crossbeam::channel::bounded::<()>(0);

        let task_ready = ready.clone();
        let mut port = self.port.try_clone()?;
        let metadata = self.metadata.clone();
        thread::spawn(move || {
            let r = || -> Result<()> {
                // Create an accumulator with the current device metadata
                let mut accumulator = MeasurementAccumulator::new(metadata);
                // First wait for main thread to clear
                // serial port input buffer
                let (lock, cvar) = &*task_ready;
                let _l = cvar
                    .wait_while(lock.lock().unwrap(), |ready| !*ready)
                    .unwrap();

                let mut buf = [0u8; 1024];
                let mut measurement_buf = VecDeque::with_capacity(100);
                loop {
                    // Check whether the main thread has signaled
                    // us to stop
                    match sig_rx.try_recv() {
                        Ok(_) => return Ok(()),
                        Err(TryRecvError::Empty) => {}
                        Err(e) => return Err(e.into()),
                    }

                    // Now we read chunks and feed them to the accumulator
                    let n = port.read(&mut buf)?;
                    accumulator.feed_into(&buf[..n], &mut measurement_buf);
                    if !measurement_buf.is_empty() {
                        measurement_buf
                            .drain(..measurement_buf.len())
                            .try_for_each(|m| meas_tx.send(m))?;
                    }
                }
            };
            let res = r();
            if let Err(e) = &res {
                tracing::error!("{:?}", e);
            };
            res
        });
        self.port.clear(Input)?;

        let (lock, cvar) = &*ready;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cvar.notify_all();

        self.send_command(Command::AverageStart)?;
        let stop = move || sig_tx.send(());
        Ok((self.into_state(), meas_rx, stop))
    }

    fn set_power_mode(&mut self, mode: PowerMode) -> Result<()> {
        self.send_command(Command::SetPowerMode(mode))?;
        Ok(())
    }
}

impl Ppk2<Measuring> {
    pub fn stop_measuring(mut self) -> Result<Ppk2<Idle>> {
        self.send_command(Command::AverageStop)?;
        Ok(self.into_state())
    }
}
