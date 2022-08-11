#![doc = include_str!("../README.md")]
use num_enum::{IntoPrimitive, TryFromPrimitive};
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
    #[error("Parse error in \"{0}\"")]
    Parse(String),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub struct Ppk2 {
    port: Box<dyn SerialPort>,
}

impl Ppk2 {
    pub fn new<'a>(path: impl Into<Cow<'a, str>>) -> Result<Self> {
        let port = serialport::new(path, 9600)
            .timeout(Duration::from_millis(10000))
            .open()?;
        Ok(Self { port })
    }

    pub fn get_metadata(&mut self) -> Result<Metadata> {
        let response = self.send_command(Command::GetMetaData)?;

        Metadata::parse(response, None)
    }

    pub fn reset(&mut self) -> Result<()> {
        self.send_command(Command::Reset)?;
        Ok(())
    }

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
}

#[derive(Debug)]
pub struct Modifiers {
    r: [f32; 5],
    gs: [f32; 5],
    gi: [f32; 5],
    o: [f32; 5],
    s: [f32; 5],
    i: [f32; 5],
    ug: [f32; 5],
}

impl Default for Modifiers {
    fn default() -> Self {
        Self {
            r: [1031.64, 101.65, 10.15, 0.94, 0.043],
            gs: [1., 1., 1., 1., 1.],
            gi: [1., 1., 1., 1., 1.],
            o: [0., 0., 0., 0., 0.],
            s: [0., 0., 0., 0., 0.],
            i: [0., 0., 0., 0., 0.],
            ug: [1., 1., 1., 1., 1.],
        }
    }
}

#[repr(u8)]
#[derive(TryFromPrimitive, IntoPrimitive, Default, Debug)]
pub enum Mode {
    #[default]
    Source = 0x01,
    Ampere = 0x02,
}

#[derive(Default, Debug)]
pub struct Metadata {
    modifiers: Modifiers,
    calibrated: bool,
    vdd: u16,
    hw: u32,
    mode: Mode,
    ia: u32,
}

impl Metadata {
    /// Example metadata:
    /// ```
    /// Calibrated: 0
    /// R0: 1003.3506
    /// R1: 101.5865
    /// R2: 10.3027
    /// R3: 0.9636
    /// R4: 0.0564
    /// GS0: 0.0000
    /// GS1: 112.7890
    /// GS2: 18.0115
    /// GS3: 2.4217
    /// GS4: 0.0729
    /// GI0: 1.0000
    /// GI1: 0.9695
    /// GI2: 0.9609
    /// GI3: 0.9519
    /// GI4: 0.9582
    /// O0: 112.9420
    /// O1: 75.4627
    /// O2: 64.6020
    /// O3: 50.4983
    /// O4: 87.2177
    /// VDD: 3000
    /// HW: 9173
    /// mode: 2
    /// S0: 0.000000048
    /// S1: 0.000000596
    /// S2: 0.000005281
    /// S3: 0.000062577
    /// S4: 0.002940743
    /// I0: -0.000000104
    /// I1: -0.000001443
    /// I2: 0.000036439
    /// I3: -0.000374119
    /// I4: -0.009388455
    /// UG0: 1.00
    /// UG1: 1.00
    /// UG2: 1.00
    /// UG3: 1.00
    /// UG4: 1.00
    /// IA: 56
    /// END
    /// ```
    #[rustfmt::skip]
    pub fn parse(bytes: Vec<u8>, metadata: Option<Metadata>) -> Result<Self> {
        use Error::Parse;

        let mut metadata = metadata.unwrap_or_default();
        let raw_metadata = String::from_utf8(bytes)?;
        if !raw_metadata.ends_with("END\n") {
            return Err(Parse(raw_metadata));
        }

        let lines = raw_metadata.lines();
        for line in lines {
            // TODO kill this beast
            match line.split_once(": ") {
                Some(("Calibrated", calibrated)) => metadata.calibrated = calibrated != "0",
                Some(("R0", r0)) => metadata.modifiers.r[0] = r0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("R1", r1)) => metadata.modifiers.r[1] = r1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("R2", r2)) => metadata.modifiers.r[2] = r2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("R3", r3)) => metadata.modifiers.r[3] = r3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("R4", r4)) => metadata.modifiers.r[4] = r4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GS0", gs0)) => metadata.modifiers.gs[0] = gs0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GS1", gs1)) => metadata.modifiers.gs[1] = gs1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GS2", gs2)) => metadata.modifiers.gs[2] = gs2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GS3", gs3)) => metadata.modifiers.gs[3] = gs3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GS4", gs4)) => metadata.modifiers.gs[4] = gs4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GI0", gi0)) => metadata.modifiers.gi[0] = gi0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GI1", gi1)) => metadata.modifiers.gi[1] = gi1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GI2", gi2)) => metadata.modifiers.gi[2] = gi2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GI3", gi3)) => metadata.modifiers.gi[3] = gi3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("GI4", gi4)) => metadata.modifiers.gi[4] = gi4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("O0", o0)) => metadata.modifiers.o[0] = o0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("O1", o1)) => metadata.modifiers.o[1] = o1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("O2", o2)) => metadata.modifiers.o[2] = o2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("O3", o3)) => metadata.modifiers.o[3] = o3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("O4", o4)) => metadata.modifiers.o[4] = o4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("VDD", vdd)) => metadata.vdd = vdd.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("HW", hw)) => metadata.hw = hw.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("mode", mode)) => metadata.mode = mode.parse::<u8>().map_err(|_| Parse(line.to_owned()))?.try_into().map_err(|_| Parse(line.to_owned()))?,
                Some(("S0", s0)) => metadata.modifiers.s[0] = s0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("S1", s1)) => metadata.modifiers.s[0] = s1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("S2", s2)) => metadata.modifiers.s[0] = s2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("S3", s3)) => metadata.modifiers.s[0] = s3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("S4", s4)) => metadata.modifiers.s[0] = s4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("I0", i0)) => metadata.modifiers.i[0] = i0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("I1", i1)) => metadata.modifiers.i[0] = i1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("I2", i2)) => metadata.modifiers.i[0] = i2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("I3", i3)) => metadata.modifiers.i[0] = i3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("I4", i4)) => metadata.modifiers.i[0] = i4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("UG0", ug0)) => metadata.modifiers.ug[0] = ug0.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("UG1", ug1)) => metadata.modifiers.ug[0] = ug1.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("UG2", ug2)) => metadata.modifiers.ug[0] = ug2.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("UG3", ug3)) => metadata.modifiers.ug[0] = ug3.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("UG4", ug4)) => metadata.modifiers.ug[0] = ug4.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("IA", ia)) => metadata.ia = ia.parse().map_err(|_| Parse(line.to_owned()))?,
                None if line == "END" => return Ok(metadata),
                _ => return Err(Parse(line.to_owned())),
            }
        }

        Ok(metadata)
    }
}
