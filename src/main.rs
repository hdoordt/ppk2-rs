use ppk2_rs::{
    types::{DevicePower, PowerMode, SourceVoltage},
    Error, Ppk2,
};
use serialport::SerialPortType::UsbPort;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

fn main() -> anyhow::Result<()> {
    // a builder for `FmtSubscriber`.
    let subscriber = FmtSubscriber::builder()
        // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
        // will be written to stdout.
        .with_max_level(Level::DEBUG)
        // completes the builder.
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
    let (ppk2, rx) = ppk2.start_measuring()?;

    loop {
        match rx.recv()? {
            Ok(m) => {
                info!("Got measurement: {m:#?}");
            }
            Err(e) => {
                error!("{e:?}");
                break;
            }
        }
    }

    // std::thread::sleep(std::time::Duration::from_secs(5));
    let ppk2 = ppk2.stop_measuring()?;
    ppk2.reset()?;
    Ok(())
}
