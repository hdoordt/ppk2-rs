use anyhow::Result;
use crossbeam::channel::RecvTimeoutError;
use ppk2::{
    types::{DevicePower, PowerMode, SourceVoltage},
    Error, Ppk2,
};
use serialport::SerialPortType::UsbPort;
use std::{collections::VecDeque, time::Duration};
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let ppk2_port = serialport::available_ports()?
        .into_iter()
        .find(|p| match &p.port_type {
            UsbPort(usb) => usb.vid == 0x1915 && usb.pid == 0xc00a,
            _ => false,
        })
        .ok_or(Error::Ppk2NotFound)?;

    let mut ppk2 = Ppk2::new(ppk2_port.port_name, PowerMode::Source)?;

    ppk2.set_source_voltage(SourceVoltage::from_millivolts(3300))?;
    ppk2.set_device_power(DevicePower::Enabled)?;
    let (ppk2, rx, kill) = ppk2.start_measuring()?;

    let mut kill = Some(kill);
    let mut data_buf = VecDeque::with_capacity(2048);

    ctrlc::set_handler(move || kill.take().unwrap()().unwrap())?;
    let r: Result<()> = loop {
        let rcv_res = rx.recv_timeout(Duration::from_millis(500));
        match rcv_res {
            Ok(msg) => match msg {
                Ok(m) => {
                    if data_buf.len() >= 2048 {
                        data_buf.pop_back();
                    }
                    data_buf.push_front(m.micro_amps);
                    let sum: f32 = data_buf.iter().sum();
                    let avg = sum / data_buf.len() as f32;
                    debug!("Last: {:.4} μA\tAverage: {avg:.4} μA", m.micro_amps);
                }

                Err(e) => {
                    warn!("Measurement missed: {e:?}");
                }
            },
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Err(e) => {
                error!("Error receiving data: {e:?}");
                break Err(e)?;
            }
        }
    };
    info!("Stopping measurements and resetting");
    ppk2.stop_measuring()?.reset()?;
    info!("Goodbye!");
    r
}
