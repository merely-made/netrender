# CLAUDE.md — Netrender Repository

Project context and architecture live in `netrender-notes/`. Start at
[`netrender-notes/PROGRESS.md`](netrender-notes/PROGRESS.md), which indexes
the surviving plans and says which are canonical.

## Git posture

**`main` has no shared ancestry with `upstream/*`.** The history was
rewritten (`sanitize archived local paths`), so `git merge-base main
upstream/main` is empty and any "N commits ahead/behind upstream" number is
meaningless. Cherry-pick only. In practice there is little worth taking:
netrender no longer contains the WebRender codebase.

**`upstream/main` is stale on purpose.** The `upstream` remote is
github.com/servo/webrender, whose `main` is pinned for Servo and last moved
2025-10-02. The live mirror of Mozilla's tree is the branch
**`upstream/upstream`** (currently tracking Firefox bug traffic). If you
need to look at real current WebRender, look there, not at `upstream/main`.

**Dead branches are annotated tags, not branches.** `origin` carries only
`main`. Everything else is under `archive/*` tags, indexed in
[`netrender-notes/archive/2026-08-10_branch_archive.md`](netrender-notes/archive/2026-08-10_branch_archive.md).
That file is the only surviving record of the wgpu-backend-for-WebRender
work aimed at servo #37149; the `webrender-wgpu-upstream` checkout it used
to live in no longer exists. Restore one with
`git checkout -b <name> archive/<name>`.

## Workspace tooling

sem and weave are wired into this repo. Both are described once in
`Code/CLAUDE.md`, which loads for any session at or below `Code`.
