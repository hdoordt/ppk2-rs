use anyhow::Result;
use clap::Parser;
use ppk2::{
    types::{DevicePower, MeasurementMode, SourceVoltage, LogicPortPins, Level},
    Ppk2, try_find_ppk2_port, measurement::MeasurementMatch,
};

use std::{
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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

    #[clap(
        long,
        value_name = "UA",
        help = "Statistics mode, also splitting each interval at this current (µA) into active and sleep parts (implies --stats)"
    )]
    split_stats: Option<f32>,

    #[clap(
        long,
        help = "Statistics mode, also writing every interval as a JSON object line (NDJSON) to stdout (implies --stats)"
    )]
    json: bool,
}

fn main() -> Result<()> {
    // Setup stuff
    let args = Args::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(args.log_level)
        .with_writer(std::io::stderr) // stdout is reserved for the --json output
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

    if args.stats || args.split_stats.is_some() || args.json {
        // The source voltage is only known (and regulated) when sourcing
        let source_mv = (args.mode == MeasurementMode::Source).then(|| args.voltage.millivolts());
        return run_stats(ppk2, args.interval_ms, source_mv, args.split_stats, args.json);
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
    /// Samples at or above the split current and their sum
    active_count: u64,
    active_sum: f64,
    /// Number of bursts: transitions from below to at or above the split current
    bursts: u64,
    in_burst: bool,
}

impl Stats {
    fn add(&mut self, micro_amps: f32, split_ua: Option<f32>) {
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

        if let Some(split) = split_ua {
            let active = micro_amps >= split;
            if active {
                self.active_count += 1;
                self.active_sum += i;
                if !self.in_burst {
                    self.bursts += 1;
                }
            }
            self.in_burst = active;
        }
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

/// Active/sleep split of one interval
struct Split {
    threshold_ua: f32,
    active_pct: f64,
    active_avg_ua: f64,
    active_charge_pct: f64,
    bursts: u64,
    burst_ms: f64,
    sleep_pct: f64,
    sleep_avg_ua: f64,
}

/// Average of `count` samples summing up to `sum`, 0 without samples
fn avg_of(sum: f64, count: f64) -> f64 {
    if count > 0. {
        sum / count
    } else {
        0.
    }
}

/// JSON number rounded to `decimals` (trailing zeros trimmed), `null` for NaN/infinite (not representable in JSON)
fn json_num(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "null".into();
    }
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".into() } else { s.into() }
}

/// Logs the statistics of one interval and, with `json`, writes them as a JSON object line to stdout
fn report(stats: &Stats, sps: usize, duration_s: f64, source_mv: Option<u16>, split_ua: Option<f32>, json: bool) {
    let n = stats.count as f64;
    let avg = stats.sum / n;
    let rms = (stats.sum_sq / n).sqrt();
    let charge_uc = avg * n / sps as f64; // µA * s = µC
    // The PPK2 regulates the source voltage, so the power is simply V * I_avg (µA * V = µW)
    let power_uw = source_mv.map(|mv| avg * f64::from(mv) / 1000.);

    let split = split_ua.map(|threshold_ua| {
        let active = stats.active_count as f64;
        let sleep = n - active;
        Split {
            threshold_ua,
            active_pct: active * 100. / n,
            active_avg_ua: avg_of(stats.active_sum, active),
            active_charge_pct: if stats.sum > 0. { stats.active_sum * 100. / stats.sum } else { 0. },
            bursts: stats.bursts,
            burst_ms: if stats.bursts > 0 { active / stats.bursts as f64 * 1000. / sps as f64 } else { 0. },
            sleep_pct: sleep * 100. / n,
            sleep_avg_ua: avg_of(stats.sum - stats.active_sum, sleep),
        }
    });

    info!(
        "avg: {avg:9.3} µA  rms: {rms:9.3} µA  min: {:9.3} µA  max: {:9.3} µA  charge: {charge_uc:9.3} µC  ({} samples in {duration_s:.3} s)",
        stats.min, stats.max, stats.count
    );
    if let Some(s) = &split {
        info!(
            "split @ {} µA: active {:6.2}% avg {:9.3} µA ({:5.1}% of charge, {} bursts of {:.3} ms avg) | sleep {:6.2}% avg {:7.3} µA",
            s.threshold_ua, s.active_pct, s.active_avg_ua, s.active_charge_pct, s.bursts, s.burst_ms, s.sleep_pct, s.sleep_avg_ua
        );
    }
    if let (Some(mv), Some(p)) = (source_mv, power_uw) {
        info!("avg_pwr: {} (at {mv} mV)", format_power(p));
    }

    if json {
        let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let mut line = format!(
            r#"{{"ts_ms":{ts_ms},"samples":{},"duration_s":{},"avg_ua":{},"rms_ua":{},"min_ua":{},"max_ua":{},"charge_uc":{}"#,
            stats.count,
            json_num(duration_s, 3),
            json_num(avg, 3),
            json_num(rms, 3),
            json_num(stats.min.into(), 3),
            json_num(stats.max.into(), 3),
            json_num(charge_uc, 3)
        );
        if let (Some(mv), Some(p)) = (source_mv, power_uw) {
            line += &format!(r#","source_mv":{mv},"avg_pwr_uw":{}"#, json_num(p, 3));
        }
        if let Some(s) = &split {
            line += &format!(
                r#","split":{{"threshold_ua":{},"active_pct":{},"active_avg_ua":{},"active_charge_pct":{},"bursts":{},"burst_ms":{},"sleep_pct":{},"sleep_avg_ua":{}}}"#,
                json_num(s.threshold_ua.into(), 3),
                json_num(s.active_pct, 2),
                json_num(s.active_avg_ua, 3),
                json_num(s.active_charge_pct, 2),
                s.bursts,
                json_num(s.burst_ms, 3),
                json_num(s.sleep_pct, 2),
                json_num(s.sleep_avg_ua, 3)
            );
        }
        line.push('}');
        println!("{line}");
    }
}

/// Sample at the full rate and print average/RMS/min/max/charge once per `interval_ms` worth of samples.
/// The interval is counted in samples, so USB buffering doesn't skew it. Its (wall clock) duration is printed
/// too: a clearly longer duration than the interval means samples were missed.
/// With a `source_mv` (source meter mode) the average power is printed as well. With a `split_ua` the
/// interval is split into active (at or above `split_ua`) and sleep parts. With `json` every interval is also
/// written to stdout as a JSON object line.
fn run_stats(
    ppk2: Ppk2,
    interval_ms: u64,
    source_mv: Option<u16>,
    split_ua: Option<f32>,
    json: bool,
) -> Result<()> {
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
                stats.add(m.micro_amps, split_ua);

                if stats.count >= samples_per_interval {
                    let duration_s = start.elapsed().as_secs_f64();
                    report(&stats, SPS, duration_s, source_mv, split_ua, json);
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
