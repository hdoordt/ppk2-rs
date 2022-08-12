use ppk2::{
    types::{DevicePower, PowerMode, SourceVoltage},
    Error, Ppk2,
};
use serialport::SerialPortType::UsbPort;
use tracing::{debug, warn, Level};
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
    let (_ppk2, rx) = ppk2.start_measuring()?;
    let mut count = 0;
    let mut count_over_1000 = 0;
    let mut count_missed = 0;
    loop {
        match rx.recv()? {
            Ok(m) => {
                count += 1;
                if m.analog_value > 1000. {
                    count_over_1000 += 1;
                }
                debug!("Got measurement: {m:#?}");
                debug!(
                    "Count: {count}. Over 1000: {}% ({}). Missed: {}% ({})",
                    100 * count_over_1000 / count,
                    count_over_1000,
                    100 * count_missed / count,
                    count_missed,
                );
            }
            Err(e) => {
                warn!("Measurement missed: {e:?}");
                count_missed += 1;
            }
        }
    }
}
