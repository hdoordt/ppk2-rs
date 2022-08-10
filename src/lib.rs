#![doc = include_str!("../README.md")]
use serialport::SerialPort;
use std::{borrow::Cow, time::Duration, io};
use thiserror::Error;

use crate::cmd::Command;

pub mod cmd;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Serial port error: {0}")]
    SerialPort(#[from] serialport::Error),
    #[error("PPK2 not found. Is the device connected and permissions set correctly?")]
    Ppk2NotFound,
    #[error("IO error: {0}")]
    Io(#[from] io::Error)
}

type Result<T> = std::result::Result<T, Error>;

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

    pub fn get_metadata(&mut self)  -> Result<()> {
        self.port.write(&[Command::GetMetaData as u8])?;
        let mut buf = [0u8; 20];
        let x = self.port.read(&mut buf);
        dbg!(x)?;
        println!("{buf:02X?}");
        Ok(())
    }
}
