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

    #[clap(
        long,
        help = "Statistics mode: sample at the full 100 kS/s (ignores --sps) and print average/RMS/min/max/charge once per interval"
    )]
    stats: bool,

    #[clap(env, short = 'i', long, help = "Statistics mode interval in ms", default_value = "1000")]
    interval_ms: u64,
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

    if args.stats {
        // The source voltage is only known (and regulated) when sourcing
        let source_mv = (args.mode == MeasurementMode::Source).then(|| args.voltage.millivolts());
        return run_stats(ppk2, args.interval_ms, source_mv);
    }

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

/// Current statistics over one interval of samples
#[derive(Default)]
struct Stats {
    count: u64,
    sum: f64,
    sum_sq: f64,
    min: f32,
    max: f32,
}

impl Stats {
    fn add(&mut self, micro_amps: f32) {
        if self.count == 0 {
            self.min = micro_amps;
            self.max = micro_amps;
        }
        let i = f64::from(micro_amps);
        self.count += 1;
        self.sum += i;
        self.sum_sq += i * i;
        self.min = self.min.min(micro_amps);
        self.max = self.max.max(micro_amps);
    }
}

/// Formats a power in µW with a fitting unit (µW, mW or W)
fn format_power(micro_watts: f64) -> String {
    if micro_watts >= 1e6 {
        format!("{:.3} W", micro_watts / 1e6)
    } else if micro_watts >= 1e3 {
        format!("{:.3} mW", micro_watts / 1e3)
    } else {
        format!("{micro_watts:.3} µW")
    }
}

/// Sample at the full rate and print average/RMS/min/max/charge once per `interval_ms` worth of samples.
/// The interval is counted in samples, so USB buffering doesn't skew it. Its (wall clock) duration is printed
/// too: a clearly longer duration than the interval means samples were missed.
/// With a `source_mv` (source meter mode) the average power is printed as well.
fn run_stats(ppk2: Ppk2, interval_ms: u64, source_mv: Option<u16>) -> Result<()> {
    const SPS: usize = 100_000;
    let (rx, kill) = ppk2.start_measurement(SPS)?;

    let mut kill = Some(kill);
    ctrlc::set_handler(move || {
        kill.take().unwrap()().unwrap();
    })?;

    let samples_per_interval = (SPS as u64 * interval_ms / 1000).max(1);
    let mut stats = Stats::default();
    let mut start = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(2000)) {
            Ok(MeasurementMatch::Match(m)) => {
                stats.add(m.micro_amps);

                if stats.count >= samples_per_interval {
                    let n = stats.count as f64;
                    let avg = stats.sum / n;
                    info!(
                        "avg: {avg:9.3} µA  rms: {:9.3} µA  min: {:9.3} µA  max: {:9.3} µA  charge: {:9.3} µC  ({} samples in {:.3} s)",
                        (stats.sum_sq / n).sqrt(),
                        stats.min,
                        stats.max,
                        avg * n / SPS as f64,
                        stats.count,
                        start.elapsed().as_secs_f64()
                    );
                    if let Some(mv) = source_mv {
                        // The PPK2 regulates the source voltage, so the power is simply V * I_avg (µA * V = µW)
                        info!("avg_pwr: {} (at {mv} mV)", format_power(avg * f64::from(mv) / 1000.));
                    }

                    stats = Stats::default();
                    start = Instant::now();
                }
            }
            Ok(MeasurementMatch::NoMatch) => {}
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Err(e) => {
                error!("Error receiving data: {e:?}");
                break Err(e)?;
            }
        }
    }
}
