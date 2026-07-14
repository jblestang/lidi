# Repro: UDP aux datagram size out of bounds

## Issue

On unpatched `master`, `diode-receive-udp` reads an 8-byte little-endian size from
the TCP/Unix side channel, then slices the internal buffer with
`buffer[0..header.size]` without validating `header.size`. Values of `0` or
`size > buffer_size` cause a panic during slice construction.

## Automated repro

```bash
cargo test repro_oversized repro_zero_and_oversized -- --nocapture
```

- `repro_oversized_datagram_slice_panics_without_bounds_check` shows the panic
  mechanism on unpatched code.
- `repro_malicious_udp_header_returns_error_without_panic` sends a crafted
  header over TCP and verifies the patched receiver returns an error instead of
  aborting.

## Manual repro (unpatched master)

1. Run `diode-receive-udp` with default `--buffer-size 4194304`.
2. Connect to its TCP side channel and send 8 bytes encoding
   `4194305` (buffer size + 1) as little-endian `u64`.
3. Observe process abort from slice bounds panic.

## Expected with this fix

Invalid sizes are rejected with `invalid datagram size …` before any buffer slice.
