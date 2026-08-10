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

## Workspace Tooling: sem & weave

Two non-authoritative structural tools from Ataraxy Labs are wired into this
repo. Both read code structure via tree-sitter, not program semantics; they
never replace `cargo check` / `cargo test` / compiling.

**weave** (entity-level git merge driver). `.gitattributes` maps ~46 file
types to `merge=weave`; ordinary `git merge` resolves false conflicts where
independent edits touch different functions, structs, or keys in the same
file. A true same-entity conflict still produces markers, tagged with the
entity name and reason (e.g. `function 'foo': both modified`). Preview a
merge before running it with `weave-cli preview <branch>`.

The merge-driver binary path is machine-local, not committed (git can't
version a local binary path). It is wired via `git config --global
merge.weave.driver` on this machine, which covers every repo including
fresh clones, so no per-repo setup is needed here. On a new machine, install
with `cargo install --git https://github.com/Ataraxy-Labs/weave weave-cli
weave-driver`, then either repeat the global `git config --global
merge.weave.*` setup or run `weave setup` in each repo.

**sem** (semantic version control): entity-level diff, context, impact, and
blame queries on top of Git. Installed via `cargo install --git
https://github.com/Ataraxy-Labs/sem sem-cli` and registered as a
user-scoped Claude Code MCP server (`sem_diff`, `sem_context`, `sem_impact`,
`sem_entities`, `sem_blame`, `sem_log`; call these directly as tools). CLI
fallback if the MCP tools are not available:

```bash
sem diff --format plain
sem context <Symbol> --budget 2000 --json
sem impact <Symbol> --file <path> --json
```

Use `sem context` and `sem impact` to brief yourself on a symbol before
editing it, especially across the sibling-repo lattice. Avoid unfiltered
scans over large directories: `sem entities crates --json` on a big tree
dumps a lot.
