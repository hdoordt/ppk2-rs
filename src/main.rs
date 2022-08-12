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
    let (ppk2, rx, sig_tx) = ppk2.start_measuring()?;

    let mut count = 0;
    let mut count_over_500 = 0;
    let mut count_missed = 0;
    let mut sig_tx = Some(sig_tx);
    let mut data = VecDeque::with_capacity(1024);

    ctrlc::set_handler(move || sig_tx.take().unwrap().send(()).unwrap())?;
    let r: Result<()> = loop {
        let rcv_res = rx.recv_timeout(Duration::from_millis(500));
        match rcv_res {
            Ok(msg) => match msg {
                Ok(m) => {
                    if data.len() >= 1024 {
                        data.pop_back();
                    }
                    data.push_front(m.analog_value);
                    let sum: f32 = data.iter().sum();
                    let avg = sum / data.len() as f32;
                    count += 1;
                    if m.analog_value > 500. {
                        count_over_500 += 1;
                    }
                    debug!("Got measurement: {m:#?}");
                    debug!(
                        "Avg: {avg}, Count: {count}. Over 500: {}% ({}). Missed: {}% ({})",
                        100 * count_over_500 / count,
                        count_over_500,
                        100 * count_missed / count,
                        count_missed,
                    );
                }

                Err(e) => {
                    warn!("Measurement missed: {e:?}");
                    count_missed += 1;
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
