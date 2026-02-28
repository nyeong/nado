use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use regex::Regex;
use serde::de::Deserializer;
use serde::Deserialize;
use wait_timeout::ChildExt;

#[derive(Parser, Debug)]
#[command(
    name = "nado",
    version,
    about = "Local differential tester for algorithm solutions"
)]
struct Cli {
    /// Path to nado TOML config
    #[arg(short, long, default_value = "tests/nado.toml")]
    config: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Config {
    #[allow(dead_code)]
    version: Option<u32>,
    problem: Problem,
    origin: Program,
    #[serde(default, deserialize_with = "deserialize_candidates")]
    candidate: Vec<Program>,
    #[serde(default)]
    engine: Engine,
    #[serde(default)]
    limits: Limits,
    #[serde(default)]
    normalize: Normalize,
}

#[derive(Debug, Deserialize)]
struct Problem {
    inputs: BTreeMap<String, InputSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct InputSpec {
    #[serde(rename = "type")]
    kind: String,
    range: Option<String>,
    min: Option<i64>,
    max: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct Program {
    name: Option<String>,
    cmd: Vec<String>,
    #[allow(dead_code)]
    image: Option<String>,
    #[serde(default)]
    mounts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Limits {
    cpu_seconds: Option<u64>,
    memory_mb: Option<u64>,
    file_size_kb: Option<u64>,
    nofile: Option<u64>,
    nproc: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CandidateField {
    One(Program),
    Many(Vec<Program>),
}

fn deserialize_candidates<'de, D>(deserializer: D) -> std::result::Result<Vec<Program>, D::Error>
where
    D: Deserializer<'de>,
{
    let field = Option::<CandidateField>::deserialize(deserializer)?;
    let Some(field) = field else {
        return Ok(Vec::new());
    };

    match field {
        CandidateField::One(program) => Ok(vec![program]),
        CandidateField::Many(programs) => Ok(programs),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Engine {
    #[serde(default = "default_cases")]
    cases: usize,
    #[serde(default = "default_seed")]
    seed: u64,
    #[serde(default = "default_workers")]
    workers: usize,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_stop_on_first_fail")]
    stop_on_first_fail: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            cases: default_cases(),
            seed: default_seed(),
            workers: default_workers(),
            timeout_ms: default_timeout_ms(),
            stop_on_first_fail: default_stop_on_first_fail(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Normalize {
    #[serde(default = "default_true")]
    trim_trailing_ws: bool,
    #[serde(default = "default_true")]
    ignore_final_newline: bool,
}

impl Default for Normalize {
    fn default() -> Self {
        Self {
            trim_trailing_ws: true,
            ignore_final_newline: true,
        }
    }
}

#[derive(Debug, Clone)]
struct ParsedInput {
    min: i64,
    max: i64,
}

#[derive(Debug)]
struct RunOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config;
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_dir = fs::canonicalize(&config_dir).unwrap_or(config_dir);

    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config: {}", config_path.display()))?;
    let config: Config = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML: {}", config_path.display()))?;

    if config.candidate.is_empty() {
        bail!("at least one candidate is required");
    }

    let parsed_inputs = parse_problem_inputs(&config.problem)
        .context("failed to parse [problem.inputs] constraints")?;

    let generated_inputs = generate_inputs(&parsed_inputs, config.engine.cases, config.engine.seed);

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

    if config.engine.stop_on_first_fail {
        let failure = pool.install(|| {
            generated_inputs
                .par_iter()
                .enumerate()
                .find_map_any(|(idx, input)| {
                    run_case(
                        idx,
                        input,
                        &config,
                        &config_dir,
                        &config.normalize,
                        &config.limits,
                        config.engine.timeout_ms,
                    )
                    .unwrap_or_else(|e| {
                        Some(Failure {
                            case_index: idx,
                            input: input.clone(),
                            candidate_name: "engine".to_string(),
                            reason: format!("runner error: {e:#}"),
                            origin_stdout: String::new(),
                            candidate_stdout: String::new(),
                            origin_stderr: String::new(),
                            candidate_stderr: String::new(),
                        })
                    })
                })
        });

        if let Some(failure) = failure {
            print_failure(&failure);
            std::process::exit(1);
        }

        println!("PASS: no mismatches found");
        return Ok(());
    }

    let mut failures = pool.install(|| {
        generated_inputs
            .par_iter()
            .enumerate()
            .filter_map(|(idx, input)| {
                run_case(
                    idx,
                    input,
                    &config,
                    &config_dir,
                    &config.normalize,
                    &config.limits,
                    config.engine.timeout_ms,
                )
                .unwrap_or_else(|e| {
                    Some(Failure {
                        case_index: idx,
                        input: input.clone(),
                        candidate_name: "engine".to_string(),
                        reason: format!("runner error: {e:#}"),
                        origin_stdout: String::new(),
                        candidate_stdout: String::new(),
                        origin_stderr: String::new(),
                        candidate_stderr: String::new(),
                    })
                })
            })
            .collect::<Vec<_>>()
    });

    failures.sort_by_key(|f| f.case_index);
    if failures.is_empty() {
        println!("PASS: no mismatches found");
        return Ok(());
    }

    println!("FAIL: {} mismatch(es)", failures.len());
    print_failure(&failures[0]);
    std::process::exit(1);
}

fn parse_problem_inputs(problem: &Problem) -> Result<Vec<ParsedInput>> {
    if problem.inputs.is_empty() {
        bail!("problem.inputs must not be empty");
    }

    let mut parsed = Vec::new();
    for (name, spec) in &problem.inputs {
        if spec.kind != "integer" {
            bail!(
                "only integer inputs are supported in MVP, got {} for {}",
                spec.kind,
                name
            );
        }

        let (min, max) = parse_bounds(spec).with_context(|| format!("input {}", name))?;
        parsed.push(ParsedInput { min, max });
    }

    Ok(parsed)
}

fn parse_bounds(spec: &InputSpec) -> Result<(i64, i64)> {
    let mut min = spec.min.unwrap_or(-100);
    let mut max = spec.max.unwrap_or(100);

    if let Some(range) = &spec.range {
        for token in range.split(&[',', '&'][..]) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }

            let Some((op, value)) = parse_constraint(token) else {
                bail!("unsupported range expression token: {token}");
            };

            match op {
                ">" => min = min.max(value + 1),
                ">=" => min = min.max(value),
                "<" => max = max.min(value - 1),
                "<=" => max = max.min(value),
                "==" => {
                    min = value;
                    max = value;
                }
                _ => bail!("unsupported operator: {op}"),
            }
        }
    }

    if min > max {
        bail!("invalid bounds: min({min}) > max({max})");
    }

    Ok((min, max))
}

fn parse_constraint(token: &str) -> Option<(&str, i64)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(<=|>=|<|>|==)\s*(-?\d+)$").expect("valid regex"));

    let caps = re.captures(token)?;
    let op = caps.get(1)?.as_str();
    let value = caps.get(2)?.as_str().parse().ok()?;
    Some((op, value))
}

fn generate_inputs(specs: &[ParsedInput], cases: usize, seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut out = Vec::with_capacity(cases);

    for _ in 0..cases {
        let line = specs
            .iter()
            .map(|s| rng.gen_range(s.min..=s.max).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        out.push(format!("{line}\n"));
    }

    out
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
    let origin = run_program(&config.origin, input, config_dir, timeout_ms, limits)
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
        let got = run_program(candidate, input, config_dir, timeout_ms, limits)
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

fn run_program(
    program: &Program,
    input: &str,
    config_dir: &Path,
    timeout_ms: u64,
    limits: &Limits,
) -> Result<RunOutput> {
    let resolved_cmd = resolve_program_cmd(program, config_dir)?;
    if resolved_cmd.is_empty() {
        bail!("program command is empty");
    }

    let mut command = Command::new(&resolved_cmd[0]);
    command
        .args(&resolved_cmd[1..])
        .current_dir(config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        let limits = limits.clone();
        unsafe {
            command.pre_exec(move || apply_limits(&limits));
        }
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn command: {}", resolved_cmd.join(" ")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .context("failed to write stdin")?;
    }

    let stdout = child.stdout.take().context("failed to capture stdout")?;
    let stderr = child.stderr.take().context("failed to capture stderr")?;

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut buf);
        buf
    });

    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut buf);
        buf
    });

    let duration = Duration::from_millis(timeout_ms);
    let mut timed_out = false;

    let status = match child.wait_timeout(duration)? {
        Some(status) => status,
        None => {
            timed_out = true;
            let _ = child.kill();
            child.wait().context("failed waiting killed process")?
        }
    };

    let stdout = stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stdout thread panicked"))?;
    let stderr = stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stderr thread panicked"))?;

    Ok(RunOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        timed_out,
    })
}

#[cfg(unix)]
fn apply_limits(limits: &Limits) -> std::io::Result<()> {
    if let Some(cpu_seconds) = limits.cpu_seconds {
        set_rlimit(libc::RLIMIT_CPU, cpu_seconds as libc::rlim_t)?;
    }
    if let Some(memory_mb) = limits.memory_mb {
        let bytes = (memory_mb * 1024 * 1024) as libc::rlim_t;
        set_rlimit(libc::RLIMIT_AS, bytes)?;
    }
    if let Some(file_size_kb) = limits.file_size_kb {
        let bytes = (file_size_kb * 1024) as libc::rlim_t;
        set_rlimit(libc::RLIMIT_FSIZE, bytes)?;
    }
    if let Some(nofile) = limits.nofile {
        set_rlimit(libc::RLIMIT_NOFILE, nofile as libc::rlim_t)?;
    }
    if let Some(nproc) = limits.nproc {
        set_rlimit(libc::RLIMIT_NPROC, nproc as libc::rlim_t)?;
    }

    Ok(())
}

#[cfg(unix)]
fn set_rlimit(resource: libc::c_int, value: libc::rlim_t) -> std::io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };

    let code = unsafe { libc::setrlimit(resource as _, &lim) };
    if code == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EINVAL) {
        // Some platforms do not support every RLIMIT resource.
        return Ok(());
    }

    Err(err)
}

fn resolve_program_cmd(program: &Program, config_dir: &Path) -> Result<Vec<String>> {
    if program.cmd.is_empty() {
        bail!("program cmd is empty");
    }

    let mount_map = build_mount_map(&program.mounts, config_dir)?;

    Ok(program
        .cmd
        .iter()
        .map(|arg| mount_map.get(arg).cloned().unwrap_or_else(|| arg.clone()))
        .collect())
}

fn build_mount_map(mounts: &[String], config_dir: &Path) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();

    for mount in mounts {
        let mut parts = mount.split(':');
        let host = parts.next().unwrap_or_default().trim();
        let container = parts.next().unwrap_or_default().trim();

        if host.is_empty() || container.is_empty() {
            bail!("invalid mount syntax: {mount}. expected host:container[:mode]");
        }

        let host_path = if Path::new(host).is_absolute() {
            PathBuf::from(host)
        } else {
            config_dir.join(host)
        };
        let host_path = fs::canonicalize(&host_path).unwrap_or(host_path);

        map.insert(
            container.to_string(),
            host_path.to_string_lossy().to_string(),
        );
    }

    Ok(map)
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

fn default_cases() -> usize {
    1000
}

fn default_seed() -> u64 {
    42
}

fn default_workers() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn default_timeout_ms() -> u64 {
    1000
}

fn default_stop_on_first_fail() -> bool {
    true
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{parse_bounds, InputSpec};

    #[test]
    fn parse_range_tokens() {
        let spec = InputSpec {
            kind: "integer".to_string(),
            range: Some(">= 1, <= 9".to_string()),
            min: None,
            max: None,
        };

        let (min, max) = parse_bounds(&spec).expect("must parse");
        assert_eq!(min, 1);
        assert_eq!(max, 9);
    }

    #[test]
    fn parse_range_with_explicit_min_max() {
        let spec = InputSpec {
            kind: "integer".to_string(),
            range: Some("> 0".to_string()),
            min: Some(5),
            max: Some(10),
        };

        let (min, max) = parse_bounds(&spec).expect("must parse");
        assert_eq!(min, 5);
        assert_eq!(max, 10);
    }
}
