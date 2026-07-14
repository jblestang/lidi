# Repro: path traversal in file receive

## Issue

On unpatched `master`, `diode-receive-file` accepts a file name of `..` from the
auxiliary file protocol header. The receiver builds the output path with
`output_dir.join("..")`, which resolves to the parent directory, so an attacker
can write files outside the intended sandbox.

## Automated repro

```bash
cargo test repro_parent_dir_filename_rejected repro_malicious_tcp_header_does_not_escape_output_dir -- --nocapture
```

## Manual repro (unpatched master)

1. Start a receiver bound to a dedicated output directory:

   ```bash
   mkdir -p /tmp/lidi-inbox
   diode-receive-file --from-tcp 127.0.0.1:9999 /tmp/lidi-inbox
   ```

2. Craft a TCP client that sends a file header whose name field is literally `..`
   (8-byte little-endian length + bytes + mode + zero file length).

3. Observe that unpatched code creates a file in `/tmp/` instead of
   `/tmp/lidi-inbox/`.

## Expected with this fix

All three tests pass: `..`, `.`, separators, and nested paths are rejected before
any file is opened.
