# nado Agent Handover (2026-02-27)

## 1) Project Goal (Current Understanding)
- Differential testing tool for BOJ-style problems.
- Given one trusted `origin` and one-or-more `candidate`s, generate test inputs and compare outputs.
- Must support untrusted candidate isolation (local limits and Docker mode), reproducible random generation, and candidate-wise verdict reporting.

## 2) Current Architecture
- Entry: `src/main.rs`
- CLI + config path resolve: `src/cli.rs`
- Config schema: `src/config.rs`
- Engine/orchestration/reporting: `src/engine.rs`
- Input parsing + generation (seeded boundary/partition + proptest random): `src/generator.rs`
- Program execution (local or Docker): `src/runner.rs`

## 3) Behavior Snapshot
- Config discovery: if first arg exists, use it; else only `./nado.toml` in current directory.
- Per case:
  - Run `origin` once.
  - Compare same expected output against all candidates.
- Candidate-wise independent verdict:
  - One candidate failure does not stop evaluating other candidates.
  - Final output prints candidate summary (`PASS`/`FAIL`/`UNKNOWN`).
- Progress bar enabled during case processing.

## 4) E2E Fixture Status
- Fixture: `tests/e2e/backjoon-1000`
- Includes:
  - origin (python)
  - candidate (ruby)
  - candidate (APL in Docker: `juergensauermann/gnu-apl:latest`)
- Engine defaults in this fixture:
  - `cases = 500`
  - `timeout_ms = 500`
  - `stop_on_first_fail = true`

## 5) Known Performance Bottlenecks
1. Docker startup per case is expensive (especially interpreted language containers).
2. Process spawn cost per case per program (origin + each candidate).
3. Generator can emit duplicate random cases when domain is small.
4. `nix run` includes build checks/caching overhead compared with running prebuilt binary directly.

## 6) Recommended Performance Plan (Priority)
1. Domain-aware dedup/exhaustive fallback
- If finite domain cardinality is small (e.g., A+B with 9x9=81), generate unique full set and stop.
- Avoid running 500 random tests for 81-state space.

2. Unique-input cache
- Deduplicate generated inputs before execution.
- Cache origin output by input key to avoid recompute if duplicates survive.

3. Warm execution mode (high impact)
- Introduce optional long-lived runner protocol for local/docker candidates.
- One process/container handles many inputs via framed stdin/stdout messages.
- Removes repeated spawn/cold-start costs.

4. Tiered budget scheduler
- Run deterministic edge/partition set first.
- Only add random batches if still all-pass and user budget remains.

5. Adaptive early stop by candidate
- Already partial: failed candidate can be skipped when `stop_on_first_fail = true`.
- Extend with per-candidate max_failures / max_runtime budget.

6. Separate local and docker concurrency pools
- Docker workers usually need lower parallelism than local CPU-bound tasks.
- Add optional `engine.workers_docker`.

## 7) Current TODO Alignment
- `TODO.md` already tracks Tier 1..4 generation and schema v2.
- Immediate additions suggested:
  - [ ] Input dedup + origin memoization
  - [ ] Small-domain exhaustive mode
  - [ ] Warm runner protocol (local/docker)

## 8) Git State at Handover
- Branch: `main`
- Recent commits:
  - `005ab16` ✨ add runtime progress bar
  - `0d6b601` ✨ modularize + docker/APL + basic PBT
- Working tree currently modified (not yet committed):
  - `src/engine.rs`
  - `README.md`

## 9) Quick Repro Commands
- Unit tests:
  - `nix develop -c cargo test`
- E2E run from fixture dir:
  - `cd tests/e2e/backjoon-1000 && nix run /Users/nyeong/Repos/nado`
- Faster local dev (skip `nix run` rebuild path where possible):
  - `nix develop -c cargo run -- tests/e2e/backjoon-1000/nado.toml`

## 10) Open Risks
- Local execution is limit-based but not full sandbox (namespace-level isolation TODO).
- Docker mode relies on daemon availability and image startup characteristics.
- Comparator still string-normalize only (float epsilon/tokenized comparator TODO).
