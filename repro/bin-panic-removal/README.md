# Repro: production binaries panic on recoverable errors

## Issue

Unpatched production binaries used `unreachable!()` and `.expect()` for
recoverable configuration/runtime failures:

- `diode-send-file` / `diode-send-udp`: no TCP/Unix destination
- `diode-receive`: `Client::try_from` with empty `Clients`
- `diode-send`: thread spawn failures

Any of these abort the whole process (`panic = "abort"` in release).

## Automated repro

```bash
cargo test -p diode --bin diode-send-file repro_
cargo test -p diode --bin diode-send-udp repro_
cargo test -p diode --bin diode-receive repro_
```

Each test constructs an empty `Clients` value (bypassing clap) and verifies the
patched code returns `None` / `InvalidInput` instead of panicking.

## Manual repro (unpatched master)

Call internal code paths with `Clients { to_tcp: None, to_unix: None }` from a
Rust integration test or patched binary — unpatched builds abort via
`unreachable!()`.

## Expected with this fix

Missing destinations and spawn failures log an error and exit cleanly.
