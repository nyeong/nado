use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{Config, Limits, Normalize};
use crate::generator::{generate_inputs, parse_problem_inputs};
use crate::runner::run_program;

#[derive(Debug, Clone)]
struct Failure {
    case_index: usize,
    input: String,
    candidate_index: Option<usize>,
    candidate_name: String,
    reason: String,
    origin_stdout: String,
    candidate_stdout: String,
    origin_stderr: String,
    candidate_stderr: String,
}

pub fn run(config_path: &Path) -> Result<i32> {
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_dir = std::fs::canonicalize(&config_dir).unwrap_or(config_dir);

    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config: {}", config_path.display()))?;
    let config: Config = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML: {}", config_path.display()))?;

    if config.candidate.is_empty() {
        bail!("at least one candidate is required");
    }

    let parsed_inputs = parse_problem_inputs(&config.problem)
        .context("failed to parse [problem.inputs] constraints")?;
    let generated_inputs = generate_inputs(
        &parsed_inputs,
        config.engine.cases,
        config.engine.seed,
        &config.pbt,
    )
    .context("failed to generate test inputs")?;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.engine.workers)
        .build()
        .context("failed to build worker pool")?;
    let candidate_names = config
        .candidate
        .iter()
        .enumerate()
        .map(|(idx, candidate)| {
            candidate
                .name
                .clone()
                .unwrap_or_else(|| format!("candidate-{}", idx + 1))
        })
        .collect::<Vec<_>>();

    println!(
        "nado: cases={}, candidates={}, workers={}, timeout={}ms",
        generated_inputs.len(),
        config.candidate.len(),
        config.engine.workers,
        config.engine.timeout_ms
    );
    let progress = build_progress_bar(generated_inputs.len());
    let failed_candidates = config.engine.stop_on_first_fail.then(|| {
        Arc::new(
            (0..config.candidate.len())
                .map(|_| AtomicBool::new(false))
                .collect::<Vec<_>>(),
        )
    });

    let progress = progress.clone();
    let mut failures = pool.install(|| {
        generated_inputs
            .par_iter()
            .enumerate()
            .flat_map_iter(|(idx, input)| {
                let result = run_case_or_failure(
                    idx,
                    input,
                    &config,
                    &config_dir,
                    &candidate_names,
                    &config.normalize,
                    &config.limits,
                    config.engine.timeout_ms,
                    failed_candidates.as_ref().map(|flags| flags.as_slice()),
                );
                progress.inc(1);
                result
            })
            .collect::<Vec<_>>()
    });

    progress.finish_and_clear();
    failures.sort_by_key(|f| (f.case_index, f.candidate_index.unwrap_or(usize::MAX)));

    let mut infra_failures = Vec::new();
    let mut candidate_failures = vec![Vec::new(); config.candidate.len()];
    for failure in failures {
        if let Some(candidate_idx) = failure.candidate_index {
            if config.engine.stop_on_first_fail && !candidate_failures[candidate_idx].is_empty() {
                continue;
            }
            candidate_failures[candidate_idx].push(failure);
        } else {
            infra_failures.push(failure);
        }
    }

    let failed_count = candidate_failures
        .iter()
        .filter(|per_candidate| !per_candidate.is_empty())
        .count();
    let has_infra_failure = !infra_failures.is_empty();

    if failed_count == 0 && !has_infra_failure {
        println!("PASS: all candidates matched origin");
        print_candidate_summary(&candidate_names, &candidate_failures, false);
        return Ok(0);
    }

    println!(
        "FAIL: {} / {} candidate(s) failed",
        failed_count,
        candidate_names.len()
    );
    print_candidate_summary(&candidate_names, &candidate_failures, has_infra_failure);

    if let Some(first_infra) = infra_failures.first() {
        println!();
        println!("origin/engine failure (candidate verdict may be incomplete):");
        print_failure(first_infra);
    }

    for per_candidate in candidate_failures.iter().filter(|f| !f.is_empty()) {
        println!();
        print_failure(&per_candidate[0]);
    }

    Ok(1)
}

fn run_case_or_failure(
    idx: usize,
    input: &str,
    config: &Config,
    config_dir: &Path,
    candidate_names: &[String],
    normalize: &Normalize,
    limits: &Limits,
    timeout_ms: u64,
    failed_candidates: Option<&[AtomicBool]>,
) -> Vec<Failure> {
    run_case(
        idx,
        input,
        config,
        config_dir,
        candidate_names,
        normalize,
        limits,
        timeout_ms,
        failed_candidates,
    )
    .unwrap_or_else(|e| vec![engine_failure(idx, input, &e)])
}

fn engine_failure(idx: usize, input: &str, error: &anyhow::Error) -> Failure {
    Failure {
        case_index: idx,
        input: input.to_string(),
        candidate_index: None,
        candidate_name: "engine".to_string(),
        reason: format!("runner error: {error:#}"),
        origin_stdout: String::new(),
        candidate_stdout: String::new(),
        origin_stderr: String::new(),
        candidate_stderr: String::new(),
    }
}

fn run_case(
    idx: usize,
    input: &str,
    config: &Config,
    config_dir: &Path,
    candidate_names: &[String],
    normalize: &Normalize,
    limits: &Limits,
    timeout_ms: u64,
    failed_candidates: Option<&[AtomicBool]>,
) -> Result<Vec<Failure>> {
    let origin_timeout_ms = config.origin.timeout_ms.unwrap_or(timeout_ms);
    let origin = run_program(&config.origin, input, config_dir, origin_timeout_ms, limits)
        .context("origin execution failed")?;

    if origin.timed_out {
        return Ok(vec![Failure {
            case_index: idx,
            input: input.to_string(),
            candidate_index: None,
            candidate_name: "origin".to_string(),
            reason: "origin timed out".to_string(),
            origin_stdout: origin.stdout,
            candidate_stdout: String::new(),
            origin_stderr: origin.stderr,
            candidate_stderr: String::new(),
        }]);
    }

    if !origin.status.success() {
        return Ok(vec![Failure {
            case_index: idx,
            input: input.to_string(),
            candidate_index: None,
            candidate_name: "origin".to_string(),
            reason: format!("origin exited with {}", origin.status),
            origin_stdout: origin.stdout,
            candidate_stdout: String::new(),
            origin_stderr: origin.stderr,
            candidate_stderr: String::new(),
        }]);
    }

    let expected = normalize_output(&origin.stdout, normalize);
    let mut failures = Vec::new();

    for (candidate_idx, candidate) in config.candidate.iter().enumerate() {
        if should_skip_candidate(failed_candidates, candidate_idx) {
            continue;
        }

        let candidate_timeout_ms = candidate.timeout_ms.unwrap_or(timeout_ms);
        let candidate_name = candidate_names[candidate_idx].clone();
        let got = match run_program(candidate, input, config_dir, candidate_timeout_ms, limits) {
            Ok(output) => output,
            Err(error) => {
                failures.push(Failure {
                    case_index: idx,
                    input: input.to_string(),
                    candidate_index: Some(candidate_idx),
                    candidate_name,
                    reason: format!("candidate runner error: {error:#}"),
                    origin_stdout: origin.stdout.clone(),
                    candidate_stdout: String::new(),
                    origin_stderr: origin.stderr.clone(),
                    candidate_stderr: String::new(),
                });
                mark_candidate_failed(failed_candidates, candidate_idx);
                continue;
            }
        };

        if got.timed_out {
            failures.push(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_index: Some(candidate_idx),
                candidate_name,
                reason: "candidate timed out".to_string(),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            });
            mark_candidate_failed(failed_candidates, candidate_idx);
            continue;
        }

        if !got.status.success() {
            failures.push(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_index: Some(candidate_idx),
                candidate_name,
                reason: format!("candidate exited with {}", got.status),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            });
            mark_candidate_failed(failed_candidates, candidate_idx);
            continue;
        }

        let actual = normalize_output(&got.stdout, normalize);
        if expected != actual {
            failures.push(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_index: Some(candidate_idx),
                candidate_name,
                reason: "output mismatch".to_string(),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            });
            mark_candidate_failed(failed_candidates, candidate_idx);
        }
    }

    Ok(failures)
}

fn normalize_output(output: &str, normalize: &Normalize) -> String {
    let mut normalized = output.replace("\r\n", "\n");

    if normalize.trim_trailing_ws {
        normalized = normalized
            .split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
    }

    if normalize.ignore_final_newline {
        normalized = normalized.trim_end_matches('\n').to_string();
    }

    normalized
}

fn print_failure(failure: &Failure) {
    println!("FAIL at case #{}", failure.case_index + 1);
    println!("candidate: {}", failure.candidate_name);
    println!("reason: {}", failure.reason);
    println!("input:\n{}", failure.input.trim_end());
    println!("origin stdout:\n{}", failure.origin_stdout.trim_end());
    println!("candidate stdout:\n{}", failure.candidate_stdout.trim_end());

    if !failure.origin_stderr.trim().is_empty() {
        println!("origin stderr:\n{}", failure.origin_stderr.trim_end());
    }
    if !failure.candidate_stderr.trim().is_empty() {
        println!("candidate stderr:\n{}", failure.candidate_stderr.trim_end());
    }
}

fn should_skip_candidate(failed_candidates: Option<&[AtomicBool]>, candidate_idx: usize) -> bool {
    let Some(failed_candidates) = failed_candidates else {
        return false;
    };
    failed_candidates[candidate_idx].load(Ordering::Relaxed)
}

fn mark_candidate_failed(failed_candidates: Option<&[AtomicBool]>, candidate_idx: usize) {
    if let Some(failed_candidates) = failed_candidates {
        failed_candidates[candidate_idx].store(true, Ordering::Relaxed);
    }
}

fn print_candidate_summary(
    candidate_names: &[String],
    candidate_failures: &[Vec<Failure>],
    has_infra_failure: bool,
) {
    println!("candidate summary:");
    for (idx, candidate_name) in candidate_names.iter().enumerate() {
        let failure_count = candidate_failures[idx].len();
        if failure_count > 0 {
            println!(
                "- {}: FAIL ({} mismatch(es))",
                candidate_name, failure_count
            );
        } else if has_infra_failure {
            println!("- {}: UNKNOWN (origin/engine failure)", candidate_name);
        } else {
            println!("- {}: PASS", candidate_name);
        }
    }
}

fn build_progress_bar(total: usize) -> ProgressBar {
    let progress = ProgressBar::new(total as u64);

    let style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar())
    .progress_chars("=>-");
    progress.set_style(style);
    progress.enable_steady_tick(Duration::from_millis(100));

    if !std::io::stdout().is_terminal() {
        progress.set_draw_target(ProgressDrawTarget::hidden());
    }

    progress
}
