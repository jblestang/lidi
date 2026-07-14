# Repro: forged Lidi block payload length

## Issue

After RaptorQ decode, unpatched `master` trusts the 4-byte `data_length` field inside
a block and slices `block[SERIALIZE_OVERHEAD..SERIALIZE_OVERHEAD + len]` without
checking that `len` fits the buffer. A forged length causes a panic (DoS) when
dispatch calls `payload()`.

## Automated repro

```bash
cargo test repro_oversized_payload repro_validate -- --nocapture
```

- `repro_oversized_payload_claim_panics_without_bounds_check` demonstrates the
  slice panic mechanism.
- `repro_validate_rejects_oversized_payload_claim` shows the patched validator
  rejecting the same forged block before dispatch.

## Manual repro (unpatched master)

1. Inject or decode a block whose declared `data_length` exceeds
   `max_data_len(raptorq)` while the actual buffer is shorter.
2. Trigger receiver dispatch (any code path calling `block.payload()`).
3. Observe process abort from bounds panic.

## Expected with this fix

`validate()` and fallible `payload()` return `InvalidPayloadLength` or
`payload out of bounds` instead of panicking.
