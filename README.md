# nado

Local differential tester for algorithm solutions.

## Why

Useful when you want to solve a problem in a language not supported by the judge.

## MVP scope

- Generate randomized inputs from `problem.inputs`
- Run `origin` and all `candidate` programs locally
- Compare normalized stdout outputs
- Fail fast with reproducible counterexample (`seed`)
- Enforce timeout + `rlimit` per process (no Docker required)

## Run

```bash
cargo run -- tests/e2e/backjoon-1000/nado.toml

# Or run inside a problem directory (auto-detect ./nado.toml)
cd tests/e2e/backjoon-1000
cargo run --manifest-path ../../../Cargo.toml
```

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

## Planning docs

- `TODO.md`
- `docs/boj-problem-survey.md`
