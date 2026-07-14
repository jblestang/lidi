# Repro: unbounded file name length in aux file protocol

## Issue

The auxiliary file protocol reads an 8-byte little-endian `file_name_len`, then
allocates `vec![0; file_name_len]` on unpatched `master`. A remote sender can
force multi-gigabyte allocations (DoS) before any content is validated.

## Automated repro

```bash
cargo test repro_huge_file_name_len -- --nocapture
```

The test crafts a header claiming a file name of `MAX_FILE_NAME_LEN + 1` bytes
without sending that many bytes; the patched parser rejects the length before
allocation.

## Manual repro (unpatched master)

1. Connect to `diode-receive-file`'s TCP side channel.
2. Send 8 bytes encoding `1_048_576` (1 MiB) as little-endian `u64` for the
   name length, followed by only a few name bytes.
3. Observe large memory allocation attempt (or OOM) on unpatched code.

## Expected with this fix

Lengths above 4096 bytes return `InvalidFileNameLen` immediately.
