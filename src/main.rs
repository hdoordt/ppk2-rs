use ppk2_rs::{Ppk2, Error};
use serialport::SerialPortType::UsbPort;

fn main() -> anyhow::Result<()> {
    let ppk2_port = serialport::available_ports()?.into_iter().find(|p| match &p.port_type {
        UsbPort(usb) => usb.vid == 0x1915 && usb.pid == 0xc00a,
        _ => false
    }).ok_or(Error::Ppk2NotFound)?;

    let mut ppk2 = Ppk2::new(ppk2_port.port_name)?;

    let metadata = ppk2.get_metadata()?;
    dbg!(metadata);
    Ok(())
}
