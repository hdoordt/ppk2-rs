use crate::types::{DevicePower, PowerMode, SourceVoltage};

#[repr(u8)]
/// Serial command opcodes
pub enum Command {
    NoOp,
    TriggerSet,
    AvgNumSet,
    TriggerWindowSet,
    TriggerIntervalSet,
    TriggerSingleSet,
    AverageStart,
    AverageStop,
    RangeSet,
    LcdSet,
    TriggerStop,
    DeviceRunningSet(DevicePower),
    RegulatorSet(SourceVoltage),
    SwitchPointDown,
    SwitchPointUp,
    TriggerExtToggle,
    SetPowerMode(PowerMode),
    ResUserSet,
    SpikeFilteringOn,
    SpikeFilteringOff,
    GetMetaData,
    Reset,
    SetUserGains,
}

impl Command {
    /// The expected length of the response, as a hint
    /// indicating how much space we should allocate for a buffer.
    /// If no specific branch for the command is defined in
    /// Command::response_complete, the expected response length
    /// is used to check whether we received the whole response.
    pub fn expected_response_len(&self) -> usize {
        match self {
            Command::NoOp => 0,
            Command::TriggerSet => 0,
            Command::AvgNumSet => 0,
            Command::TriggerWindowSet => 0,
            Command::TriggerIntervalSet => 0,
            Command::TriggerSingleSet => 0,
            Command::AverageStart => 0,
            Command::AverageStop => 0,
            Command::RangeSet => 0,
            Command::LcdSet => 0,
            Command::TriggerStop => 0,
            Command::DeviceRunningSet(_) => 0,
            Command::RegulatorSet(_) => 0,
            Command::SwitchPointDown => 0,
            Command::SwitchPointUp => 0,
            Command::TriggerExtToggle => 0,
            Command::SetPowerMode(_) => 0,
            Command::ResUserSet => 0,
            Command::SpikeFilteringOn => 0,
            Command::SpikeFilteringOff => 0,
            Command::GetMetaData => 512,
            Command::Reset => 0,
            Command::SetUserGains => 0,
        }
    }

    pub fn response_complete(&self, response: &[u8]) -> bool {
        match self {
            Command::GetMetaData => response.ends_with(b"END\n"),
            _ => self.expected_response_len() >= response.len(),
        }
    }
}

impl Command {
    pub fn bytes<'b>(&'b self) -> CommandBytes<'b> {
        CommandBytes {
            cmd: self,
            index: 0,
        }
    }
}

pub struct CommandBytes<'c> {
    cmd: &'c Command,
    index: usize,
}

impl<'c> Iterator for CommandBytes<'c> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        use Command::*;
        let b = match (self.cmd, self.index) {
            (NoOp, 0) => Some(0x00),
            (TriggerSet, 0) => Some(0x01),
            (AvgNumSet, 0) => Some(0x02),
            (TriggerWindowSet, 0) => Some(0x03),
            (TriggerIntervalSet, 0) => Some(0x04),
            (TriggerSingleSet, 0) => Some(0x05),
            (AverageStart, 0) => Some(0x06),
            (AverageStop, 0) => Some(0x07),
            (RangeSet, 0) => Some(0x08),
            (LcdSet, 0) => Some(0x09),
            (TriggerStop, 0) => Some(0x0A),
            (DeviceRunningSet(_), 0) => Some(0x0C),
            (DeviceRunningSet(pwr), 1) => Some((*pwr).into()),
            (RegulatorSet(_), 0) => Some(0x0D),
            (RegulatorSet(vdd), i) if (1..=2).contains(&i) => Some(vdd.raw()[i - 1]),
            (SwitchPointDown, 0) => Some(0x0E),
            (SwitchPointUp, 0) => Some(0x0F),
            (TriggerExtToggle, 0) => Some(0x10),
            (SetPowerMode(_), 0) => Some(0x11),
            (SetPowerMode(mode), 1) => Some((*mode).into()),
            (ResUserSet, 0) => Some(0x12),
            (SpikeFilteringOn, 0) => Some(0x15),
            (SpikeFilteringOff, 0) => Some(0x16),
            (GetMetaData, 0) => Some(0x19),
            (Reset, 0) => Some(0x20),
            (SetUserGains, 0) => Some(0x25),
            _ => None,
        };
        self.index += 1;
        b
    }
}
