#![doc = include_str!("../README.md")]
use serialport::SerialPort;
use std::{borrow::Cow, io, ops::Range, string::FromUtf8Error, time::Duration};
use thiserror::Error;

use crate::cmd::Command;

const VDD_RANGE_MILLIVOLTS: Range<u16> = Range {
    start: 800,
    end: 5000,
};

pub mod cmd;

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
    #[error("Parse")]
    Parse(String),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub struct Ppk2 {
    port: Box<dyn SerialPort>,
}

impl Ppk2 {
    pub fn new<'a>(path: impl Into<Cow<'a, str>>) -> Result<Self> {
        let port = serialport::new(path, 9600)
            .timeout(Duration::from_millis(500))
            .open()?;
        Ok(Self { port })
    }

    pub fn get_metadata(&mut self) -> Result<Modifiers> {
        let response = self.send_command(Command::GetMetaData)?;

        Modifiers::parse(response, None)
    }

    pub fn reset(&mut self) -> Result<()> {
        self.send_command(Command::Reset)?;
        Ok(())
    }

    pub fn send_command(&mut self, command: Command) -> Result<Vec<u8>> {
        self.port.write_all(&Vec::from_iter(command.bytes()))?;
        // Doesn't allocate if expected response length is 0
        let mut buf = Vec::with_capacity(command.expected_response_len());
        self.port.read_exact(&mut buf)?;
        Ok(buf)
    }
}

#[derive(Debug)]
pub struct Modifiers {
    r: [f32; 5],
    gs: [u8; 5],
    gi: [u8; 5],
    o: [u8; 5],
    s: [u8; 5],
    i: [u8; 5],
    ug: [u8; 5],
}

impl Modifiers {
    pub fn merge(&mut self, incomplete: IncompleteModifiers) {
        merge!(r, incomplete);  
    }
}

impl Default for Modifiers {
    fn default() -> Self {
        Self {
            r: [1031.64, 101.65, 10.15, 0.94, 0.043],
            gs: [1, 1, 1, 1, 1],
            gi: [1, 1, 1, 1, 1],
            o: [0, 0, 0, 0, 0],
            s: [0, 0, 0, 0, 0],
            i: [0, 0, 0, 0, 0],
            ug: [1, 1, 1, 1, 1],
        }
    }
}

impl Modifiers {
    pub fn parse(bytes: Vec<u8>, modifiers: Option<Modifiers>) -> Result<Self> {
        use Error::Parse;

        let mut modifiers = modifiers.unwrap_or_default();

        let metadata = String::from_utf8(bytes)?;
        dbg!(&metadata); // TODO may be JSON
        if !metadata.ends_with("END") {
            return Err(Parse(metadata));
        }
        let lines = metadata.lines();
        for line in lines {
            todo!("PArse line by line")
        }

        todo!();
    }
}
