use anyhow::Result;
use clap::Parser;
use ppk2::{
    types::{DevicePower, MeasurementMode, SourceVoltage, LogicPortPins, Level},
    Ppk2, try_find_ppk2_port, measurement::MeasurementMatch,
};

use std::{
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};
use tracing::{debug, error, info, Level as LogLevel};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
struct Args {
    #[clap(
        env,
        short = 'p',
        long,
        help = "The serial port the PPK2 is connected to. If unspecified, will try to find the PPK2 automatically"
    )]
    serial_port: Option<String>,

    #[clap(
        env,
        short = 'v',
        long,
        help = "The voltage of the device source in mV",
        default_value = "0"
    )]
    voltage: SourceVoltage,

    #[clap(
        env,
        short = 'e',
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
    log_level: LogLevel,

    #[clap(
        env,
        short = 's',
        long,
        help = "The maximum number of samples to be taken per second. Uses averaging of device samples Samples are analyzed in chunks, and as such the actual number of samples per second will deviate",
        default_value = "100"
    )]
    sps: usize,
}

fn main() -> Result<()> {
    // Setup stuff
    let args = Args::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(args.log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let ppk2_port = match args.serial_port {
        Some(p) => p,
        None => try_find_ppk2_port()?,
    };

    // Connect to PPK2 and initialize
    let mut ppk2 = Ppk2::new(ppk2_port, args.mode)?;
    ppk2.set_source_voltage(args.voltage)?;
    ppk2.set_device_power(args.power)?;

    // Set up pin pattern for matching
    // This particular setup will only
    // match measurements if pin 0 is low.
    let mut levels = [Level::Either; 8];
    levels[0] = Level::Low;
    let pins = LogicPortPins::with_levels(levels);

    // Start measuring.
    let (rx, kill) = ppk2.start_measurement_matching(pins, args.sps)?;

    // Set up sigkill handler.
    let mut kill = Some(kill);
    ctrlc::set_handler(move || {
        kill.take().unwrap()().unwrap();
    })?;

    // Receive measurements
    let mut count = 0usize;
    let start = Instant::now();
    let r: Result<()> = loop {
        let rcv_res = rx.recv_timeout(Duration::from_millis(2000));
        count += 1;
        use MeasurementMatch::*;
        match rcv_res {
            Ok(Match(m)) => {
                debug!("Last chunk average: {:.4} μA", m.micro_amps);
            }
            Ok(NoMatch) => {
                debug!("No match in the last chunk of measurements");
            }
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Err(e) => {
                error!("Error receiving data: {e:?}");
                break Err(e)?;
            }
        }
    };
    let sample_time = Instant::now().duration_since(start).as_secs() as usize;
    info!("Samples per second: {}", count / sample_time);
    info!("Stopping measurements and resetting");
    info!("Goodbye!");
    r
}
