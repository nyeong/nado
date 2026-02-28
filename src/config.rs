use std::collections::BTreeMap;

use serde::de::Deserializer;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[allow(dead_code)]
    pub version: Option<u32>,
    pub problem: Problem,
    pub origin: Program,
    #[serde(default, deserialize_with = "deserialize_candidates")]
    pub candidate: Vec<Program>,
    #[serde(default)]
    pub engine: Engine,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub pbt: Pbt,
    #[serde(default)]
    pub normalize: Normalize,
}

#[derive(Debug, Deserialize)]
pub struct Problem {
    pub inputs: BTreeMap<String, InputSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InputSpec {
    #[serde(rename = "type")]
    pub kind: String,
    pub range: Option<String>,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Program {
    pub name: Option<String>,
    pub cmd: Vec<String>,
    pub image: Option<String>,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub mounts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Limits {
    pub cpu_seconds: Option<u64>,
    pub memory_mb: Option<u64>,
    pub file_size_kb: Option<u64>,
    pub nofile: Option<u64>,
    pub nproc: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pbt {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_edge_case_ratio")]
    pub edge_case_ratio: f64,
    #[serde(default = "default_partition_ratio")]
    pub partition_ratio: f64,
    #[serde(default = "default_max_cartesian_cases")]
    pub max_cartesian_cases: usize,
}

impl Default for Pbt {
    fn default() -> Self {
        Self {
            enabled: true,
            edge_case_ratio: default_edge_case_ratio(),
            partition_ratio: default_partition_ratio(),
            max_cartesian_cases: default_max_cartesian_cases(),
        }
    }
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
pub struct Engine {
    #[serde(default = "default_cases")]
    pub cases: usize,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_stop_on_first_fail")]
    pub stop_on_first_fail: bool,
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
pub struct Normalize {
    #[serde(default = "default_true")]
    pub trim_trailing_ws: bool,
    #[serde(default = "default_true")]
    pub ignore_final_newline: bool,
}

impl Default for Normalize {
    fn default() -> Self {
        Self {
            trim_trailing_ws: true,
            ignore_final_newline: true,
        }
    }
}

fn default_cases() -> usize {
    1000
}

fn default_seed() -> u64 {
    42
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
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

fn default_edge_case_ratio() -> f64 {
    0.2
}

fn default_partition_ratio() -> f64 {
    0.2
}

fn default_max_cartesian_cases() -> usize {
    128
}
