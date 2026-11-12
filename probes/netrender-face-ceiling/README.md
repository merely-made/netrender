# netrender-face-ceiling

L0a face-throughput ceiling: bodies as axis-aligned rect sets under one affine,
measured in static, live and fragment modes at 1920x1080 (medians plus
`last_frame_timings()`).

```sh
cd probes/netrender-face-ceiling
cargo run --release              # writes l0a_raw.json (or $L0A_OUT)
python make_receipt.py           # turns the raw JSON into the receipt
```

It has its own workspace root and lock; the Netrender workspace and its lock are
untouched. Origin: Mere's former `crates/probes/netrender-face-ceiling`, a local
probe that was never tracked.
