use anyhow::{bail, Result};
use clap::Parser;
use ppk2::{
    types::{DevicePower, MeasurementMode, SourceVoltage},
    Error, Ppk2,
};
use serialport::SerialPortType::UsbPort;
use std::{
    collections::VecDeque, io::Write, path::PathBuf, str::FromStr, sync::mpsc::RecvTimeoutError,
    time::Duration,
};
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
struct Args {
    #[clap(
        env,
        short = 's',
        long,
        help = "The serial port the PPK2 is connected to. If unspecified, will try to find the PPK2 automatically"
    )]
    serial_port: Option<String>,

    #[clap(
        env,
        short = 'v',
        long,
        help = "The voltage of the device source in mV"
    )]
    voltage: SourceVoltage,

    #[clap(
        env,
        short = 'p',
        long,
        help = "Enable power",
        default_value = "disabled"
    )]
    power: DevicePower,

    #[clap(
        env,
        short = 'm',
        long,
        help = "Measurement mode",
        default_value = "source"
    )]
    mode: MeasurementMode,

    #[clap(env, short = 'l', long, help = "The log level", default_value = "info")]
    log_level: Level,

    #[clap(
        env,
        short = 'f',
        long,
        help = "The file to write the measurement data to. Please make sure the folder exists, but the file itself does not."
    )]
    file: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(args.log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let log_file_path = args
        .file;
    if log_file_path.is_dir() {
        bail!("Log file path is not a file.");
    }
    if log_file_path.exists() {
        bail!("Log file already exists.")
    }
    let mut log_file = std::fs::File::create(log_file_path)?;

    let ppk2_port = match args.serial_port {
        Some(p) => p,
        None => try_find_ppk2_port()?,
    };

    let mut ppk2 = Ppk2::new(ppk2_port, args.mode)?;

    ppk2.set_source_voltage(args.voltage)?;
    ppk2.set_device_power(args.power)?;
    let (rx, kill) = ppk2.start_measuring()?;

    let mut kill = Some(kill);
    let mut data_buf = VecDeque::with_capacity(2048);

    ctrlc::set_handler(move || {
        kill.take().unwrap()().unwrap();
    })?;
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
                    writeln!(&mut log_file, "{:.4}", m.micro_amps)?;
                }

                Err(e) => {
                    warn!("Measurement missed: {e:?}");
                    writeln!(
                        &mut log_file,
                        "?{}-{}",
                        e.expected_counter.map(|c| c as i16).unwrap_or(-1),
                        e.actual_counter
                    )?;
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
    info!("Goodbye!");
    r
}

/// Try to find the serial port the PPK2 is connected to.
fn try_find_ppk2_port() -> Result<String> {
    Ok(serialport::available_ports()?
        .into_iter()
        .find(|p| match &p.port_type {
            UsbPort(usb) => usb.vid == 0x1915 && usb.pid == 0xc00a,
            _ => false,
        })
        .ok_or(Error::Ppk2NotFound)?
        .port_name)
}
