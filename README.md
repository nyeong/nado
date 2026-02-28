# nado

Local differential tester for algorithm solutions.

## Why

Useful when you want to solve a problem in a language not supported by the judge.

## MVP scope

- Generate randomized inputs from `problem.inputs`
- Run `origin` and all `candidate` programs locally by default
- Run a program in Docker when `image` is set
- Compare normalized stdout outputs
- Candidate-wise verdicts (one candidate failure does not stop others)
- Enforce timeout + `rlimit` per local process
- Generate inputs with hybrid strategy:
  - boundary seeds
  - partition seeds
  - proptest-based random samples

## Run

```bash
cargo run -- tests/e2e/backjoon-1000/nado.toml

# Or run inside a problem directory (auto-detect ./nado.toml)
cd tests/e2e/backjoon-1000
cargo run --manifest-path ../../../Cargo.toml
```

APL docker candidate in `tests/e2e/backjoon-1000` uses:

- image: `juergensauermann/gnu-apl:latest`
- script: `solve.apl`

## Nix

```bash
nix develop
cargo test
cargo run -- tests/e2e/backjoon-1000/nado.toml

# Build binary
nix build
./result/bin/nado tests/e2e/backjoon-1000/nado.toml

# Run directly
cd tests/e2e/backjoon-1000
nix run /Users/nyeong/Repos/nado

# Optional explicit config path (first arg)
nado ./nado.toml
```

## Config example

See: `tests/e2e/backjoon-1000/nado.toml`

### PBT options

`[pbt]` is optional. Defaults are enabled and tuned for generic BOJ-style integer constraints.

```toml
[pbt]
enabled = true
edge_case_ratio = 0.25
partition_ratio = 0.15
max_cartesian_cases = 128
```

## Planning docs

- `TODO.md`
- `docs/boj-problem-survey.md`
