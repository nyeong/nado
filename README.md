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
cargo run -- --config tests/nado.toml
```

## Nix

```bash
nix develop
cargo test
cargo run -- --config tests/nado.toml

# Build binary
nix build
./result/bin/nado --config tests/nado.toml

# Run directly
nix run . -- --config tests/nado.toml
```

## Config example

See: `tests/nado.toml`
