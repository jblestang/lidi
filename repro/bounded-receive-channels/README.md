# Repro: unbounded receiver channel memory growth

## Issue

On unpatched `master`, the receiver pipeline (`reblock → decode → dispatch`) and
per-client block queues use `crossbeam_channel::unbounded()`. A sender that
outpaces decoding can enqueue an arbitrary number of blocks in RAM (memory DoS).

## Automated repro

```bash
cargo test repro_bounded_pipeline repro_unbounded_queue -- --nocapture
```

The tests contrast patched `bounded(PIPELINE_QUEUE_DEPTH)` behavior (returns
`TrySendError::Full`) with unpatched `unbounded()` behavior (never full).

## Manual repro (unpatched master)

1. Run `diode-receive` with a slow decode path (or attach a debugger to stall
   decode workers).
2. Flood the sender with many parallel transfers.
3. Observe resident memory grow linearly with queued blocks (no backpressure).

## Expected with this fix

Pipeline depth is capped at `WINDOW_WIDTH + 1` (128 + 1) and per-client queues
use the same bound, forcing send-side blocking instead of unbounded buffering.
