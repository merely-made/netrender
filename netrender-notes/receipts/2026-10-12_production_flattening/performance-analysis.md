# Identity-matched production replay timing analysis

All twelve fixed-input replays passed pixel acceptance. Paired comparisons require identical bundle SHA, capture digest, panel dimensions, source bindings, resource payload identities and thresholds.

Times are milliseconds. Ratios are medians of paired case ratios.

| Authored width | Backend | Old encode median (range) | New encode median (range) | Paired ratio median (range) |
|---|---|---:|---:|---:|
| 1280 | cpu | 938.49 (842.30–1032.01) | 1307.64 (1155.45–1700.91) | 1.41 (1.23–1.80) |
| 1280 | classic | Unavailable | 37.21 (35.94–55.29) | Unavailable |
| 1280 | gpu | 306.31 (291.79–372.87) | 653.81 (584.81–962.60) | 2.11 (1.61–3.12) |
| 420 | cpu | 326.14 (319.44–356.00) | 505.82 (503.02–609.79) | 1.56 (1.44–1.86) |
| 420 | classic | Unavailable | 21.34 (20.37–23.19) | Unavailable |
| 420 | gpu | 175.08 (170.02–178.01) | 344.21 (331.01–356.96) | 1.97 (1.94–2.03) |
| all | cpu | 599.15 (319.44–1032.01) | 882.62 (503.02–1700.91) | 1.52 (1.23–1.86) |
| all | classic | Unavailable | 29.57 (20.37–55.29) | Unavailable |
| all | gpu | 234.90 (170.02–372.87) | 470.89 (331.01–962.60) | 2.01 (1.61–3.12) |

## Initialization and bounded readback

| Authored width | Backend | Phase | Old median (range) | New median (range) | Paired ratio median (range) |
|---|---|---|---:|---:|---:|
| 1280 | cpu | initialize_ms | 0.01 (0.01–0.01) | 0.05 (0.05–0.05) | 4.83 (4.66–5.23) |
| 1280 | classic | initialize_ms | Unavailable | 78.55 (76.65–125.53) | Unavailable |
| 1280 | classic | bounded_submit_readback_ms | Unavailable | 85.58 (81.98–90.94) | Unavailable |
| 1280 | gpu | initialize_ms | 106.30 (103.27–174.27) | 110.06 (107.56–233.31) | 1.04 (0.65–2.00) |
| 1280 | gpu | bounded_submit_readback_ms | 12.85 (6.80–17.40) | 11.27 (6.53–14.42) | 0.76 (0.43–2.12) |
| 420 | cpu | initialize_ms | 0.01 (0.01–0.01) | 0.05 (0.05–0.05) | 4.78 (4.38–5.35) |
| 420 | classic | initialize_ms | Unavailable | 80.56 (78.00–81.71) | Unavailable |
| 420 | classic | bounded_submit_readback_ms | Unavailable | 56.45 (52.13–58.36) | Unavailable |
| 420 | gpu | initialize_ms | 104.12 (101.63–105.53) | 106.96 (106.47–111.07) | 1.03 (1.01–1.07) |
| 420 | gpu | bounded_submit_readback_ms | 5.58 (3.67–10.55) | 7.89 (3.72–10.04) | 1.05 (0.83–1.92) |

## Measurement limits

- One timing observation per backend/subject/case; each render creates a fresh session/device. Labels cold/repeat describe capture history, not warm renderer-session measurements.
- CPU render_encode_submit_ms measures synchronous CPU raster. Classic/SparseGPU values measure host render/encode/submit calls, not device execution. GPU initialization includes adapter/device boot.
- bounded_submit_readback_ms includes copy submission, bounded queue completion, mapping/copying and Classic quality polling; it is not a GPU timestamp or isolated shader time.
- Every paired comparison matches bundle SHA, capture digest, panel PNG dimensions, producer bindings, resource metadata/payload digests and thresholds. Sparse adapter metadata matches. Receipt provider label changes from bound-source-scene-cpu-raster to caller-authoritative-complete-bundle because new run uses --bundle; fixed bundle bytes are identical.
- No matching production Classic baseline exists. Earlier wide-cold Classic receipt differs in bundle SHA, capture digest and bindings, so no Classic regression ratio is reported.
- These observations expose candidate-batch cost without isolating flatness from other source changes, timing variance or thermal state.

Per-case values and both producer-source summaries are preserved in `performance-analysis.json`.
