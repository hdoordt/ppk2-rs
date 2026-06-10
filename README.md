# PPK2-rs

Rust library and CLI for working with Nordic Semiconductor's Power Profiler Kit II. Far from complete, but can be used at least with the PPK2 functioning as a source meter.

Heavily based on [nrfconnect-ppk](https://github.com/NordicSemiconductor/pc-nrfconnect-ppk) and its derivate [ppk2-api-python](https://github.com/IRNAS/ppk2-api-python).

## Usage
In any case, please make sure to install the required `udev` rules first by running
```sh
./setup-udev.sh
```

### Using the CLI
If you want to quickly take some measurements, you can execute the `cli` example.

To get an overview of its arguments, run
```sh
cargo run --example cli -- --help
```

### Using as a library
Simply add `ppk2` as a dependency in `Cargo.toml`:

```toml
[dependencies]
ppk2 = "<replace-with-version>"
```

Please refer to [`examples/cli.rs`](examples/cli.rs) to get an idea of how to use it.
