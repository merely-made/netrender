# Archive

This directory holds historical notes that are still useful for traceability but
are no longer the primary working set.

## Subdirectories

- `progress/`
  - dated execution snapshots and pass-count reports
- `legacy/`
  - older branch-shape plans, debug notes, and diagnostics archives

## Dead source links are expected

Most files here describe work on the GL-era WebRender tree, so they link
to paths like `../webrender/src/device/gl.rs` that no longer exist. That
tree was removed from `main`. To follow one of those links, check out the
`archive/webrender-0.62-pre-phase-d` tag and read it there. A few docs
also point into a `graphshell/` sibling repo that has since been
consolidated away.

These are not being repaired. A dead link to a deleted file is a truthful
record of what the doc was written against; rewriting it to point
somewhere else would not be.

Branch-level history has the same shape: every retired branch is now an
annotated `archive/*` tag, indexed in
[`2026-08-10_branch_archive.md`](2026-08-10_branch_archive.md).

For the current working set, start from `../PROGRESS.md`.
