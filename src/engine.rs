use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Config, Limits, Normalize};
use crate::generator::{generate_inputs, parse_problem_inputs};
use crate::runner::run_program;

#[derive(Debug)]
struct Failure {
    case_index: usize,
    input: String,
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

    println!(
        "nado: cases={}, candidates={}, workers={}, timeout={}ms",
        generated_inputs.len(),
        config.candidate.len(),
        config.engine.workers,
        config.engine.timeout_ms
    );
    let progress = build_progress_bar(generated_inputs.len());

    if config.engine.stop_on_first_fail {
        let progress = progress.clone();
        let failure = pool.install(|| {
            generated_inputs
                .par_iter()
                .enumerate()
                .find_map_any(|(idx, input)| {
                    let result = run_case_or_failure(
                        idx,
                        input,
                        &config,
                        &config_dir,
                        &config.normalize,
                        &config.limits,
                        config.engine.timeout_ms,
                    );
                    progress.inc(1);
                    result
                })
        });

        if let Some(failure) = failure {
            progress.finish_and_clear();
            print_failure(&failure);
            return Ok(1);
        }

        progress.finish_and_clear();
        println!("PASS: no mismatches found");
        return Ok(0);
    }

    let progress = progress.clone();
    let mut failures = pool.install(|| {
        generated_inputs
            .par_iter()
            .enumerate()
            .filter_map(|(idx, input)| {
                let result = run_case_or_failure(
                    idx,
                    input,
                    &config,
                    &config_dir,
                    &config.normalize,
                    &config.limits,
                    config.engine.timeout_ms,
                );
                progress.inc(1);
                result
            })
            .collect::<Vec<_>>()
    });

    progress.finish_and_clear();
    failures.sort_by_key(|f| f.case_index);
    if failures.is_empty() {
        println!("PASS: no mismatches found");
        return Ok(0);
    }

    println!("FAIL: {} mismatch(es)", failures.len());
    print_failure(&failures[0]);
    Ok(1)
}

fn run_case_or_failure(
    idx: usize,
    input: &str,
    config: &Config,
    config_dir: &Path,
    normalize: &Normalize,
    limits: &Limits,
    timeout_ms: u64,
) -> Option<Failure> {
    run_case(
        idx, input, config, config_dir, normalize, limits, timeout_ms,
    )
    .unwrap_or_else(|e| Some(engine_failure(idx, input, &e)))
}

fn engine_failure(idx: usize, input: &str, error: &anyhow::Error) -> Failure {
    Failure {
        case_index: idx,
        input: input.to_string(),
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
    normalize: &Normalize,
    limits: &Limits,
    timeout_ms: u64,
) -> Result<Option<Failure>> {
    let origin_timeout_ms = config.origin.timeout_ms.unwrap_or(timeout_ms);
    let origin = run_program(&config.origin, input, config_dir, origin_timeout_ms, limits)
        .context("origin execution failed")?;

    if origin.timed_out {
        return Ok(Some(Failure {
            case_index: idx,
            input: input.to_string(),
            candidate_name: "origin".to_string(),
            reason: "origin timed out".to_string(),
            origin_stdout: origin.stdout,
            candidate_stdout: String::new(),
            origin_stderr: origin.stderr,
            candidate_stderr: String::new(),
        }));
    }

    if !origin.status.success() {
        return Ok(Some(Failure {
            case_index: idx,
            input: input.to_string(),
            candidate_name: "origin".to_string(),
            reason: format!("origin exited with {}", origin.status),
            origin_stdout: origin.stdout,
            candidate_stdout: String::new(),
            origin_stderr: origin.stderr,
            candidate_stderr: String::new(),
        }));
    }

    let expected = normalize_output(&origin.stdout, normalize);

    for (candidate_idx, candidate) in config.candidate.iter().enumerate() {
        let candidate_timeout_ms = candidate.timeout_ms.unwrap_or(timeout_ms);
        let got = run_program(candidate, input, config_dir, candidate_timeout_ms, limits)
            .with_context(|| format!("candidate {} execution failed", candidate_idx + 1))?;

        let candidate_name = candidate
            .name
            .clone()
            .unwrap_or_else(|| format!("candidate-{}", candidate_idx + 1));

        if got.timed_out {
            return Ok(Some(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_name,
                reason: "candidate timed out".to_string(),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            }));
        }

        if !got.status.success() {
            return Ok(Some(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_name,
                reason: format!("candidate exited with {}", got.status),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            }));
        }

        let actual = normalize_output(&got.stdout, normalize);
        if expected != actual {
            return Ok(Some(Failure {
                case_index: idx,
                input: input.to_string(),
                candidate_name,
                reason: "output mismatch".to_string(),
                origin_stdout: origin.stdout.clone(),
                candidate_stdout: got.stdout,
                origin_stderr: origin.stderr.clone(),
                candidate_stderr: got.stderr,
            }));
        }
    }

    Ok(None)
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
