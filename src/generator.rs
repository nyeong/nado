use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config as RunnerConfig, RngAlgorithm, TestRng, TestRunner};
use regex::Regex;

use crate::config::{InputSpec, Pbt, Problem};

#[derive(Debug, Clone)]
pub struct ParsedInput {
    pub min: i64,
    pub max: i64,
}

pub fn parse_problem_inputs(problem: &Problem) -> Result<Vec<ParsedInput>> {
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

pub fn generate_inputs(
    specs: &[ParsedInput],
    cases: usize,
    seed: u64,
    pbt: &Pbt,
) -> Result<Vec<String>> {
    if cases == 0 {
        return Ok(Vec::new());
    }

    let mut seeded_cases = Vec::new();

    if pbt.enabled {
        validate_pbt_config(pbt)?;
        let edge_budget = ((cases as f64) * pbt.edge_case_ratio).round() as usize;
        let partition_budget = ((cases as f64) * pbt.partition_ratio).round() as usize;

        extend_edge_cases(
            &mut seeded_cases,
            specs,
            edge_budget,
            pbt.max_cartesian_cases,
        );
        extend_partition_cases(&mut seeded_cases, specs, partition_budget);
    }

    if seeded_cases.len() > cases {
        seeded_cases.truncate(cases);
    }

    let random_needed = cases.saturating_sub(seeded_cases.len());
    let random_cases = generate_random_cases(specs, random_needed, seed)?;

    let mut all_cases = seeded_cases;
    all_cases.extend(random_cases);

    Ok(all_cases
        .into_iter()
        .map(|values| format_case(&values))
        .collect())
}

fn validate_pbt_config(pbt: &Pbt) -> Result<()> {
    if !(0.0..=1.0).contains(&pbt.edge_case_ratio) {
        bail!("pbt.edge_case_ratio must be between 0.0 and 1.0");
    }
    if !(0.0..=1.0).contains(&pbt.partition_ratio) {
        bail!("pbt.partition_ratio must be between 0.0 and 1.0");
    }
    if pbt.edge_case_ratio + pbt.partition_ratio > 1.0 {
        bail!("pbt.edge_case_ratio + pbt.partition_ratio must be <= 1.0");
    }
    if pbt.max_cartesian_cases == 0 {
        bail!("pbt.max_cartesian_cases must be > 0");
    }

    Ok(())
}

fn extend_edge_cases(
    out: &mut Vec<Vec<i64>>,
    specs: &[ParsedInput],
    budget: usize,
    max_cartesian_cases: usize,
) {
    if budget == 0 || specs.is_empty() {
        return;
    }

    let mids = specs
        .iter()
        .map(|s| midpoint(s.min, s.max))
        .collect::<Vec<_>>();

    push_unique(out, mids.clone(), budget);
    push_unique(out, specs.iter().map(|s| s.min).collect::<Vec<_>>(), budget);
    push_unique(out, specs.iter().map(|s| s.max).collect::<Vec<_>>(), budget);

    let alt_min_max = specs
        .iter()
        .enumerate()
        .map(|(i, s)| if i % 2 == 0 { s.min } else { s.max })
        .collect::<Vec<_>>();
    push_unique(out, alt_min_max, budget);

    let alt_max_min = specs
        .iter()
        .enumerate()
        .map(|(i, s)| if i % 2 == 0 { s.max } else { s.min })
        .collect::<Vec<_>>();
    push_unique(out, alt_max_min, budget);

    for (idx, spec) in specs.iter().enumerate() {
        for edge in edge_values(spec) {
            let mut candidate = mids.clone();
            candidate[idx] = edge;
            push_unique(out, candidate, budget);

            if out.len() >= budget {
                return;
            }
        }
    }

    let edge_sets = specs.iter().map(edge_values).collect::<Vec<_>>();
    let total_cartesian = edge_sets
        .iter()
        .fold(1usize, |acc, set| acc.saturating_mul(set.len()));

    if total_cartesian == 0 || total_cartesian > max_cartesian_cases {
        return;
    }

    let mut stack = Vec::with_capacity(specs.len());
    cartesian_collect(&edge_sets, 0, &mut stack, out, budget);
}

fn cartesian_collect(
    edge_sets: &[Vec<i64>],
    depth: usize,
    stack: &mut Vec<i64>,
    out: &mut Vec<Vec<i64>>,
    budget: usize,
) {
    if out.len() >= budget {
        return;
    }

    if depth == edge_sets.len() {
        push_unique(out, stack.clone(), budget);
        return;
    }

    for &value in &edge_sets[depth] {
        if out.len() >= budget {
            return;
        }

        stack.push(value);
        cartesian_collect(edge_sets, depth + 1, stack, out, budget);
        stack.pop();
    }
}

fn extend_partition_cases(out: &mut Vec<Vec<i64>>, specs: &[ParsedInput], budget: usize) {
    if budget == 0 || specs.is_empty() {
        return;
    }

    let partition_values = specs.iter().map(partition_points).collect::<Vec<_>>();

    let mut cursor = 0usize;
    while out.len() < budget {
        let mut values = Vec::with_capacity(specs.len());

        for (idx, points) in partition_values.iter().enumerate() {
            let point = points[(cursor + idx) % points.len()];
            values.push(point);
        }

        push_unique(out, values, budget);
        cursor += 1;

        if cursor > budget.saturating_mul(4) {
            break;
        }
    }
}

fn generate_random_cases(specs: &[ParsedInput], count: usize, seed: u64) -> Result<Vec<Vec<i64>>> {
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut runner = build_proptest_runner(seed);
    let mut out = Vec::with_capacity(count);

    for _ in 0..count {
        let mut values = Vec::with_capacity(specs.len());

        for spec in specs {
            let strategy = spec.min..=spec.max;
            let tree = strategy.new_tree(&mut runner).map_err(|e| {
                anyhow::anyhow!("failed to generate random case from strategy: {e}")
            })?;
            values.push(tree.current());
        }

        out.push(values);
    }

    Ok(out)
}

fn build_proptest_runner(seed: u64) -> TestRunner {
    let seed_bytes = seed_to_bytes(seed);
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes);
    TestRunner::new_with_rng(RunnerConfig::default(), rng)
}

fn seed_to_bytes(seed: u64) -> [u8; 32] {
    let mut out = [0u8; 32];

    for i in 0..4 {
        let part = seed.wrapping_add(i as u64).to_le_bytes();
        let start = i * 8;
        out[start..start + 8].copy_from_slice(&part);
    }

    out
}

fn edge_values(spec: &ParsedInput) -> Vec<i64> {
    let mut values = BTreeSet::new();

    for candidate in [
        spec.min,
        spec.min.saturating_add(1),
        spec.max.saturating_sub(1),
        spec.max,
        0,
        1,
        -1,
    ] {
        if (spec.min..=spec.max).contains(&candidate) {
            values.insert(candidate);
        }
    }

    values.into_iter().collect()
}

fn partition_points(spec: &ParsedInput) -> Vec<i64> {
    let mut values = BTreeSet::new();

    values.insert(spec.min);
    values.insert(interpolate(spec.min, spec.max, 1, 4));
    values.insert(midpoint(spec.min, spec.max));
    values.insert(interpolate(spec.min, spec.max, 3, 4));
    values.insert(spec.max);

    if (spec.min..=spec.max).contains(&0) {
        values.insert(0);
    }

    values.into_iter().collect()
}

fn midpoint(min: i64, max: i64) -> i64 {
    interpolate(min, max, 1, 2)
}

fn interpolate(min: i64, max: i64, num: i64, den: i64) -> i64 {
    let min128 = i128::from(min);
    let max128 = i128::from(max);
    let delta = max128 - min128;
    let value = min128 + (delta * i128::from(num)) / i128::from(den);
    value as i64
}

fn push_unique(out: &mut Vec<Vec<i64>>, values: Vec<i64>, budget: usize) {
    if out.len() >= budget {
        return;
    }

    if !out.iter().any(|existing| existing == &values) {
        out.push(values);
    }
}

fn format_case(values: &[i64]) -> String {
    let body = values
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{body}\n")
}

pub fn parse_bounds(spec: &InputSpec) -> Result<(i64, i64)> {
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

#[cfg(test)]
mod tests {
    use crate::config::{InputSpec, Pbt, Problem};
    use std::collections::BTreeMap;

    use super::{generate_inputs, parse_bounds, parse_problem_inputs};

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

    #[test]
    fn seeded_generation_includes_edges() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "a".to_string(),
            InputSpec {
                kind: "integer".to_string(),
                range: None,
                min: Some(1),
                max: Some(9),
            },
        );
        inputs.insert(
            "b".to_string(),
            InputSpec {
                kind: "integer".to_string(),
                range: None,
                min: Some(1),
                max: Some(9),
            },
        );

        let problem = Problem { inputs };
        let specs = parse_problem_inputs(&problem).expect("parse");

        let samples = generate_inputs(&specs, 30, 42, &Pbt::default()).expect("generate");
        assert!(samples.iter().any(|line| line.trim() == "1 1"));
        assert!(samples.iter().any(|line| line.trim() == "9 9"));
    }
}
