# TODO

## Current Status

- [x] Local differential runner (origin vs candidates)
- [x] `nix run` support with auto-discovery of `./nado.toml`
- [x] E2E baseline for BOJ 1000

## P0 - Blockers for More BOJ Coverage

- [ ] Input schema v2 for dependent shapes (`N` then `N` values, repeated testcases `T`)
- [ ] String generators (uppercase/lowercase alphabet, digit strings, whitespace-preserving line)
- [ ] Fixed-size integer vector generators (`count = 8`, `count = 9`)
- [ ] Comparator modes (`exact`, `float_epsilon`, `tokenized`) per problem
- [ ] Output budget guard (truncate + fail when stdout exceeds limit)

## P1 - Runner Hardening

- [ ] Kill child process group on timeout (avoid orphan grandchildren)
- [ ] Linux sandbox backend integration (nsjail/isolate) behind feature flag
- [ ] Collect per-case telemetry (`elapsed_ms`, `exit_code`, timeout count)

## P1 - Test Suite Expansion

- [ ] Add e2e fixture for BOJ 1152 (word count with whitespace edge cases)
- [ ] Add 10 additional researched fixtures from `docs/boj-problem-survey.md`
- [ ] Add "known wrong candidate" for each fixture to verify mismatch reporting

## P2 - UX & Config

- [ ] Validate TOML schema with human-readable diagnostics
- [ ] `nado init` command to scaffold `nado.toml` + origin/candidate templates
- [ ] `nado list` command to discover fixtures under `tests/e2e`
