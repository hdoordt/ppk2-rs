# PPK2-rs

Rust library and CLI for working with Nordic Semiconductor's Power Profiler Kit II. Far from complete, but can be used at least with the PPK2 functioning as a source meter.

Heavily based on [nrfconnect-ppk](https://github.com/NordicSemiconductor/pc-nrfconnect-ppk) and its derivate [ppk2-api-python](https://github.com/IRNAS/ppk2-api-python).

## Library

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
ppk2 = "0.1"
```

[`examples/cli.rs`](examples/cli.rs) shows how to connect to the PPK2, configure it and receive measurements.

## CLI

The CLI is the `cli` example of this repository. Run it from a checkout, preferably as a release build: in statistics mode it handles every single sample (100 kS/s) and a debug build may not keep up.

```sh
cargo run --release --example cli -- [OPTIONS]
```

### Options

| Option | Default | Description |
|---|---|---|
| `-p`, `--serial-port <PORT>` | auto detect | Serial port of the PPK2, e.g. `COM27` or `/dev/ttyACM0` |
| `-v`, `--voltage <MV>` | `0` | Source voltage in mV, clamped to 800..5000 mV |
| `-e`, `--power <POWER>` | `disabled` | Power the device under test: `enabled` (`e`) or `disabled` (`d`) |
| `-m`, `--mode <MODE>` | `source` | `source` (`s`): source meter, the PPK2 powers the device. `ampere` (`amp`, `a`): ammeter, measure the current of an external supply |
| `-l`, `--log-level <LEVEL>` | `info` | `trace`, `debug`, `info`, `warn` or `error` |
| `-s`, `--sps <SPS>` | `100` | Averaging mode: number of averaged values reported per second |
| `--stats` | | Statistics mode (see below) |
| `-i`, `--interval-ms <MS>` | `1000` | Statistics mode interval |
| `--split-stats <UA>` | | Statistics mode, split every interval at this current into active and sleep parts (implies `--stats`) |
| `--json` | | Statistics mode, write every interval as JSON to stdout (implies `--stats`) |

All options can also be set with an environment variable, e.g. `VOLTAGE=3000`, see `--help`.

The PPK2 always samples at 100 kS/s, the modes only differ in how the samples are reported. Missed samples (detected by the sample counter of the PPK2) are logged as a warning.

### Averaging mode (default)

Averages every chunk of `100000 / sps` samples into one value. The values are logged at debug level, so use `-l debug` to see them:

```sh
cargo run --release --example cli -- -v 3000 -e enabled -l debug
```

This mode only uses the samples taken while logic input D0 is low (an example of matching on the logic port, see `examples/cli.rs`). With nothing connected to D0 that is all of them.

### Statistics mode

Uses every sample and reports once per interval (counted in samples, so USB buffering doesn't skew it):

```sh
cargo run --release --example cli -- -v 3000 -e enabled --stats
```

```text
INFO cli: avg:   371.735 µA  rms:  3171.789 µA  min:     1.463 µA  max: 104842.477 µA  charge:  1115.204 µC  (300000 samples in 2.999 s)
INFO cli: avg_pwr: 1.859 mW (at 5000 mV)
```

- `avg`: average current, determines the battery life.
- `rms`: root mean square current, determines the losses in series resistances (e.g. a battery's internal resistance). Much higher than `avg` for bursty loads.
- `min`/`max`: lowest and highest sample, e.g. for sizing a battery or buffer capacitor.
- `charge`: charge drawn during the interval.
- The duration shows how long the interval actually took: clearly longer than the interval means samples were missed.
- `avg_pwr`: average power (source voltage × average current). Source meter mode only, as the supply voltage is unknown in ammeter mode.

### Active/sleep split

`--split-stats <UA>` splits every interval at the given current into an active part (at or above) and a sleep part:

```sh
cargo run --release --example cli -- -v 3000 -e enabled -i 3000 --split-stats 1000
```

```text
INFO cli: split @ 1000 µA: active   1.02% avg  34810.500 µA ( 94.3% of charge, 3 bursts of 3.400 ms avg) | sleep  98.98% avg  20.480 µA
```

For each part: the share of time and average current, for the active part also its share of the charge, the number of bursts (transitions into active) and their average length. This shows where the charge goes, e.g. whether an increase comes from more or longer active periods or from the sleep current. Choose the split current well above any switching regulator pulses to only count the real activity.

### JSON output

`--json` writes every interval as one JSON object per line (NDJSON) to stdout, all logging goes to stderr:

```sh
cargo run --release --example cli -- -v 5000 -e enabled -i 3000 --split-stats 200 --json > measurement.ndjson
```

JSON output (formatted here for readability)

```json
{
    "ts_ms": 1789456693840,
    "samples": 300000,
    "duration_s": 2.999,
    "avg_ua": 371.735,
    "rms_ua": 3171.789,
    "min_ua": 1.463,
    "max_ua": 104842.477,
    "charge_uc": 1115.204,
    "source_mv": 5000,
    "avg_pwr_uw": 1858.674,
    "split": {
        "threshold_ua": 200,
        "active_pct": 8.17,
        "active_avg_ua": 4455.72,
        "active_charge_pct": 97.96,
        "bursts": 45,
        "burst_ms": 5.449,
        "sleep_pct": 91.83,
        "sleep_avg_ua": 8.242
    }
}
```

| Field | Unit | Description |
|---|---|---|
| `ts_ms` | ms | Unix timestamp at the end of the interval |
| `samples` | | Number of samples in the interval |
| `duration_s` | s | Actual duration of the interval |
| `avg_ua`, `rms_ua`, `min_ua`, `max_ua` | µA | Current statistics |
| `charge_uc` | µC | Charge drawn during the interval |
| `source_mv`, `avg_pwr_uw` | mV, µW | Source voltage and average power (source meter mode only) |
| `split` | | Active/sleep split (with `--split-stats` only): `threshold_ua`, `active_pct`, `active_avg_ua`, `active_charge_pct`, `bursts`, `burst_ms`, `sleep_pct`, `sleep_avg_ua` |

Numbers are rounded to a sensible resolution (3 decimals, 2 for percentages), values that can't be represented in JSON (NaN, infinite) are `null`.

## USB permissions (Linux)

On Linux the PPK2 shows up as a serial port (`/dev/ttyACM*`) that normal users can't access by default. Add a udev rule for the PPK2 (USB vendor `1915`, product `c00a`) to give access to the logged in user (`uaccess`) and the `plugdev` group:

```sh
echo 'ATTRS{idVendor}=="1915", ATTRS{idProduct}=="c00a", MODE="660", GROUP="plugdev", TAG+="uaccess"' \
    | sudo tee /etc/udev/rules.d/99-ppk2.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then disconnect and reconnect the PPK2. Check the result with `ls -l /dev/ttyACM*`.

If your user is not in the `plugdev` group (and you don't rely on `uaccess`), add it with `sudo usermod -aG plugdev $USER` and log in again.

Windows and macOS need no setup: the PPK2 shows up as `COMx` or `/dev/tty.usbmodem*` respectively.
