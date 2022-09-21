//! Several utility types used to communicate with the device.

use std::{fmt::Display, num::ParseIntError, str::FromStr};

use crate::{Error, Result};
use num_enum::{IntoPrimitive, TryFromPrimitive};

/// Error parsing one of the types defined by this crate.
#[derive(Debug)]
pub struct ParseTypeError(String, &'static str);

impl std::error::Error for ParseTypeError {}

impl Display for ParseTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error parsing: expected one of {}, but got {}",
            self.1, self.0
        )
    }
}

#[derive(Default, Debug)]
/// Device source voltage.
pub struct SourceVoltage {
    raw: [u8; 2],
}

impl FromStr for SourceVoltage {
    type Err = ParseIntError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mv = s.parse()?;
        Ok(Self::from_millivolts(mv))
    }
}

impl SourceVoltage {
    const VDD_MIN_MV: u16 = 800;
    const VDD_MAX_MV: u16 = 5000;
    const OFFSET: u16 = 32;

    /// Create a [SourceVoltage] from the passed amount of millivolts.
    pub fn from_millivolts(mv: u16) -> Self {
        let mv = mv.clamp(Self::VDD_MIN_MV, Self::VDD_MAX_MV);

        let diff_to_baseline = mv - Self::VDD_MIN_MV + Self::OFFSET;

        let ratio = (diff_to_baseline / 256) as u8;
        let remainder = (diff_to_baseline % 256) as u8;

        Self {
            raw: [ratio + 3, remainder],
        }
    }

    pub(crate) fn raw(&self) -> &[u8; 2] {
        &self.raw
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Modifiers {
    pub(crate) r: [f32; 5],
    pub(crate) gs: [f32; 5],
    pub(crate) gi: [f32; 5],
    pub(crate) o: [f32; 5],
    pub(crate) s: [f32; 5],
    pub(crate) i: [f32; 5],
    pub(crate) ug: [f32; 5],
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
#[derive(TryFromPrimitive, IntoPrimitive, Debug, Default, Clone, Copy, PartialEq, Eq)]
/// Device current measurement mode
pub enum MeasurementMode {
    /// Act as ammeter, measuring the current through the
    /// VIN and GND pins.
    Ampere = 0x01,
    #[default]
    /// Act as source meter, returing the current supplied
    /// from the device voltage source.
    Source = 0x02,
}

impl FromStr for MeasurementMode {
    type Err = ParseTypeError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ampere" | "amp" | "a" => Ok(Self::Ampere),
            "source" | "s" => Ok(Self::Source),
            _ => Err(ParseTypeError(
                s.to_owned(),
                "[ampere | amp | a | source | s]",
            )),
        }
    }
}

#[repr(u8)]
#[derive(TryFromPrimitive, IntoPrimitive, Debug, Default, Clone, Copy, PartialEq, Eq)]
/// Device power
pub enum DevicePower {
    #[default]
    /// Device is disabled
    Disabled = 0x00,
    /// Device is enabled
    Enabled = 0x01,
}

impl FromStr for DevicePower {
    type Err = ParseTypeError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "disabled" | "d" => Ok(Self::Disabled),
            "enabled" | "e" => Ok(Self::Enabled),
            _ => Err(ParseTypeError(s.to_owned(), "[disabled | d | enabled | e]")),
        }
    }
}

/// Logic level for logic port pins
#[derive(Debug, Clone, Copy, Default)]
pub enum Level {
    /// Low level
    Low,
    /// High level
    High,
    /// Either level. Used for matching only.
    #[default]
    Either,
}

impl From<bool> for Level {
    fn from(level: bool) -> Self {
        use Level::*;
        match level {
            true => High,
            false => Low,
        }
    }
}

impl Level {
    /// Check whether the level is high.
    pub fn is_high(&self) -> bool {
        matches!(self, Level::High)
    }

    /// Check whether the level is low.
    pub fn is_low(&self) -> bool {
        matches!(self, Level::Low)
    }

    /// Check whether the [Level] matches another.
    pub fn matches(&self, other: Level) -> bool {
        match (self, other) {
            (_, Level::Either) => true,
            (Level::Either, _) => true,
            (Level::Low, Level::Low) => true,
            (Level::High, Level::High) => true,
            _ => false,
        }
    }
}

/// Logic port state
#[derive(Debug, Clone, Copy, Default)]
pub struct LogicPortPins {
    pin_levels: [Level; 8],
}

impl LogicPortPins {
    /// Set a pin level
    pub fn set_level(mut self, pin: usize, level: Level) -> Self {
        self.pin_levels[pin] = level;
        self
    }

    /// Set up a new [LogicPortPins] with given [Level]s
    pub fn with_levels(pin_levels: [Level; 8]) -> Self {
        Self { pin_levels }
    }

    /// Check whether a pin level is high
    pub fn pin_is_high(&self, pin: usize) -> bool {
        self.pin_levels[pin].is_high()
    }

    /// Check whether a pin level is low
    pub fn pin_is_low(&self, pin: usize) -> bool {
        !self.pin_is_high(pin)
    }

    /// Get a reference to the internal pin array
    pub fn inner(&self) -> &[Level; 8] {
        &self.pin_levels
    }
}

impl From<[bool; 8]> for LogicPortPins {
    fn from(pin_bools: [bool; 8]) -> Self {
        let mut pins = [Level::Low; 8];
        pin_bools.iter().enumerate().for_each(|(i, &p)| {
            pins[i] = p.into();
        });
        Self { pin_levels: pins }
    }
}

impl From<u8> for LogicPortPins {
    fn from(inner: u8) -> Self {
        let mut pins = [false; 8];
        for (i, pin) in pins.iter_mut().enumerate() {
            *pin = inner & 1 << i != 0
        }
        pins.into()
    }
}

impl From<u32> for LogicPortPins {
    fn from(v: u32) -> Self {
        (v as u8).into()
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
/// parsed device metadata
pub struct Metadata {
    pub(crate) modifiers: Modifiers,
    /// Whether or not the device was calibrated
    pub calibrated: bool,
    /// Device source voltage setting
    pub vdd: u16,
    #[allow(missing_docs)]
    pub hw: u32,
    /// Device measurement mode
    pub mode: MeasurementMode,
    #[allow(missing_docs)]
    pub ia: u32,
}

impl Metadata {
    /// Example metadata:
    /// ```notest
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
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        use Error::Parse;

        let mut metadata = Metadata::default();
        let raw_metadata = std::str::from_utf8(bytes)?;
        if !raw_metadata.ends_with("END\n") {
            return Err(Parse(raw_metadata.to_owned()));
        }

        let lines = raw_metadata.lines();
        for line in lines {
            // TODO kill this beast
            match line.split_once(": ") {
                Some(("Calibrated", calibrated)) => metadata.calibrated = calibrated != "0",
                Some(("R0", r0)) => {
                    metadata.modifiers.r[0] = r0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("R1", r1)) => {
                    metadata.modifiers.r[1] = r1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("R2", r2)) => {
                    metadata.modifiers.r[2] = r2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("R3", r3)) => {
                    metadata.modifiers.r[3] = r3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("R4", r4)) => {
                    metadata.modifiers.r[4] = r4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GS0", gs0)) => {
                    metadata.modifiers.gs[0] = gs0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GS1", gs1)) => {
                    metadata.modifiers.gs[1] = gs1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GS2", gs2)) => {
                    metadata.modifiers.gs[2] = gs2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GS3", gs3)) => {
                    metadata.modifiers.gs[3] = gs3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GS4", gs4)) => {
                    metadata.modifiers.gs[4] = gs4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GI0", gi0)) => {
                    metadata.modifiers.gi[0] = gi0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GI1", gi1)) => {
                    metadata.modifiers.gi[1] = gi1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GI2", gi2)) => {
                    metadata.modifiers.gi[2] = gi2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GI3", gi3)) => {
                    metadata.modifiers.gi[3] = gi3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("GI4", gi4)) => {
                    metadata.modifiers.gi[4] = gi4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("O0", o0)) => {
                    metadata.modifiers.o[0] = o0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("O1", o1)) => {
                    metadata.modifiers.o[1] = o1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("O2", o2)) => {
                    metadata.modifiers.o[2] = o2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("O3", o3)) => {
                    metadata.modifiers.o[3] = o3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("O4", o4)) => {
                    metadata.modifiers.o[4] = o4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("VDD", vdd)) => {
                    metadata.vdd = vdd.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("HW", hw)) => metadata.hw = hw.parse().map_err(|_| Parse(line.to_owned()))?,
                Some(("mode", mode)) => {
                    metadata.mode = mode
                        .parse::<u8>()
                        .map_err(|_| Parse(line.to_owned()))?
                        .try_into()
                        .map_err(|_| Parse(line.to_owned()))?
                }
                Some(("S0", s0)) => {
                    metadata.modifiers.s[0] = s0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("S1", s1)) => {
                    metadata.modifiers.s[1] = s1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("S2", s2)) => {
                    metadata.modifiers.s[2] = s2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("S3", s3)) => {
                    metadata.modifiers.s[3] = s3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("S4", s4)) => {
                    metadata.modifiers.s[4] = s4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("I0", i0)) => {
                    metadata.modifiers.i[0] = i0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("I1", i1)) => {
                    metadata.modifiers.i[1] = i1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("I2", i2)) => {
                    metadata.modifiers.i[2] = i2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("I3", i3)) => {
                    metadata.modifiers.i[3] = i3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("I4", i4)) => {
                    metadata.modifiers.i[4] = i4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("UG0", ug0)) => {
                    metadata.modifiers.ug[0] = ug0.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("UG1", ug1)) => {
                    metadata.modifiers.ug[1] = ug1.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("UG2", ug2)) => {
                    metadata.modifiers.ug[2] = ug2.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("UG3", ug3)) => {
                    metadata.modifiers.ug[3] = ug3.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("UG4", ug4)) => {
                    metadata.modifiers.ug[4] = ug4.parse().map_err(|_| Parse(line.to_owned()))?
                }
                Some(("IA", ia)) => metadata.ia = ia.parse().map_err(|_| Parse(line.to_owned()))?,
                None if line == "END" => return Ok(metadata),
                _ => return Err(Parse(line.to_owned())),
            }
        }

        Ok(metadata)
    }
}

#[cfg(test)]
mod tests {

    use crate::types::Metadata;

    use super::{MeasurementMode, Modifiers};

    #[test]
    #[ignore = "assert_eq! doesn't work for floats, need to find another solution"]
    pub fn get_adc_result() {
        let raw_metadata = r#"Calibrated: 0
R0: 1003.3506
R1: 101.5865
R2: 10.3027
R3: 0.9636
R4: 0.0564
GS0: 0.0000
GS1: 112.7890
GS2: 18.0115
GS3: 2.4217
GS4: 0.0729
GI0: 1.0000
GI1: 0.9695
GI2: 0.9609
GI3: 0.9519
GI4: 0.9582
O0: 112.9420
O1: 75.4627
O2: 64.6020
O3: 50.4983
O4: 87.2177
VDD: 3741
HW: 9173
mode: 2
S0: 0.000000048
S1: 0.000000596
S2: 0.000005281
S3: 0.000062577
S4: 0.002940743
I0: -0.000000104
I1: -0.000001443
I2: 0.000036439
I3: -0.000374119
I4: -0.009388455
UG0: 1.00
UG1: 1.00
UG2: 1.00
UG3: 1.00
UG4: 1.00
IA: 56
END
"#;
        let metadata =
            Metadata::from_bytes(raw_metadata.as_bytes()).expect("Error parsing metadata");

        let expected_modifiers = Modifiers {
            r: [1003.3506, 101.5865, 10.3027, 0.9636, 0.0564],
            gs: [0.0000, 112.7890, 18.0115, 2.4217, 0.0729],
            gi: [1.0000, 0.9695, 0.9609, 0.9519, 0.9582],
            o: [112.9420, 75.4627, 64.6020, 50.4983, 87.2177],
            s: [
                0.000000048,
                0.000000596,
                0.000005281,
                0.000062577,
                0.002940743,
            ],
            i: [
                -0.000000104,
                -0.000001443,
                0.000036439,
                -0.000374119,
                -0.009388455,
            ],
            ug: [1.00, 1.00, 1.00, 1.00, 1.00],
        };

        let expected_metadata = Metadata {
            modifiers: expected_modifiers,
            calibrated: false,
            vdd: 3714,
            hw: 9173,
            mode: MeasurementMode::Source,
            ia: 56,
        };

        assert_eq!(expected_metadata, metadata);
    }
}
