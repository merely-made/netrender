# Distinct font backing and face admission

Sparse text planning now admits face metadata once per distinct immutable Arc
backing allocation and collection index in each frame. Glyph runs can reuse the
parsed FontRef, units-per-em and glyph count without repeating head/maxp,
unsupported-table, variation-axis-count and outline-format checks. Face admission
is frame-local; retained Blob IDs do not bypass next-frame admission.

A separate frame-local identity index checks every distinct (Blob ID, backing)
against an accepted canonical backing for that ID. Different allocations with
the same ID still compare actual bytes and refuse differences, including aliases
encountered after another run has admitted the face. Pointer equality applies
only to the same live immutable Arc allocation. The scene and retained state
keep those allocations alive for the complete plan; pointer keys are never
serialized or persisted. Retained fonts are already an accepted, internally
consistent set, so one canonical backing per retained ID suffices.

Font accounting continues to use the existing ID-plus-face FontKey. Sharing one
backing under different IDs does not collapse font count/byte budgets. Each run
still resolves its own variation location and checks size/transforms; each
positioned glyph still checks its ID, outline/bounds and aggregate segment work.
Planning remains transactional, and cache commit/clear/epoch behavior is unchanged.
The same owner serves CPU, native Hybrid and browser sparse text; native consumer
performance remains separately qualified.

The work-count regression genuinely fails before memoization: 64 repeated runs
perform 64 face admissions instead of one. The frozen failing binary/source and
log are retained under `/Users/markik/Code/output/cue-font-admission/`. Eight unit
regressions cover once-per-frame work, shared backings under different IDs,
equal-byte aliases in distinct backings, retained/same-frame identity collisions,
collection faces, and later glyph/transform refusal. A counted AsRef alias checks
one byte comparison and one face parse per frame despite repeated uses and
retained fonts. All eight pass after the change. All five portable CPU VB3 text
tests also pass, preserving variation, malformed-font, outline-budget and
cache-lifecycle behavior. This source slice starts at the exact Cue supplier
pin 79c9487; it does not select newer Vello work.

The motivating [Cue renderer profile](https://github.com/merely-made/wgpu-cue/blob/feafdcb1c87f6bcad1eeeb13e8afc82a1e58c8a2/docs/2026-11-04-renderer-submission-profile.md)
separates font-byte comparison, preparation and glyph path lowering. This fix
changes font admission only; atlas behavior, visibility decisions, image caches,
GPU submission and process/frame-slot lifetime contracts remain with their owners.
