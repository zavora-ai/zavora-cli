use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::retrieval::{LocalFileRetrievalService, RetrievalService, RetrievedChunk, query_terms};
use crate::telemetry::{TelemetrySink, unix_ms_now};

pub const DEFAULT_EVAL_DATASET_PATH: &str = "evals/datasets/retrieval-baseline.v1.json";
pub const DEFAULT_EVAL_OUTPUT_PATH: &str = ".zavora/evals/latest.json";

#[derive(Debug, Deserialize)]
pub struct EvalDataset {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub cases: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub query: String,
    pub chunks: Vec<String>,
    #[serde(default)]
    pub required_terms: Vec<String>,
    #[serde(default = "default_eval_max_chunks")]
    pub max_chunks: usize,
    pub min_term_matches: Option<usize>,
}

fn default_eval_max_chunks() -> usize {
    3
}

#[derive(Debug, Serialize)]
pub struct EvalCaseReport {
    pub id: String,
    pub passed: bool,
    pub required_terms: usize,
    pub matched_terms: usize,
    pub retrieved_chunks: usize,
    pub top_score: usize,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct EvalRunReport {
    pub generated_at_unix_ms: u128,
    pub dataset_name: String,
    pub dataset_version: String,
    pub dataset_description: String,
    pub benchmark_iterations: usize,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub pass_rate: f64,
    pub fail_under: f64,
    pub passed_threshold: bool,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub throughput_qps: f64,
    pub case_reports: Vec<EvalCaseReport>,
}

pub fn load_eval_dataset(path: &str) -> Result<EvalDataset> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read eval dataset at '{}'", path))?;
    let dataset = serde_json::from_str::<EvalDataset>(&content)
        .with_context(|| format!("invalid eval dataset json at '{}'", path))?;
    if dataset.cases.is_empty() {
        return Err(anyhow::anyhow!(
            "eval dataset '{}' has no cases; add at least one case",
            path
        ));
    }
    Ok(dataset)
}

pub fn normalize_eval_terms(raw_terms: &[String], query: &str) -> Vec<String> {
    let mut terms = if raw_terms.is_empty() {
        query_terms(query)
    } else {
        raw_terms
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect::<Vec<String>>()
    };

    terms.sort();
    terms.dedup();
    terms
}

pub fn percentile(values: &[f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let pct = pct.clamp(0.0, 100.0);
    let rank = ((pct / 100.0) * ((values.len() - 1) as f64)).round() as usize;
    values[rank.min(values.len() - 1)]
}

pub fn round_metric(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub fn run_eval_harness(
    dataset: &EvalDataset,
    benchmark_iterations: usize,
    fail_under: f64,
) -> Result<EvalRunReport> {
    let iterations = benchmark_iterations.max(1);
    let suite_start = Instant::now();
    let mut passed_cases = 0usize;
    let mut latency_ms = Vec::<f64>::new();
    let mut case_reports = Vec::<EvalCaseReport>::new();

    for case in &dataset.cases {
        if case.id.trim().is_empty() {
            return Err(anyhow::anyhow!("eval dataset contains case with empty id"));
        }
        if case.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' has empty query; each case must include query",
                case.id
            ));
        }
        if case.chunks.is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' has no chunks; each case must include retrieval corpus chunks",
                case.id
            ));
        }

        let retrieval = LocalFileRetrievalService {
            chunks: case
                .chunks
                .iter()
                .enumerate()
                .map(|(idx, chunk)| RetrievedChunk {
                    source: format!("eval:{}#{}", case.id, idx + 1),
                    text: chunk.clone(),
                    score: 0,
                })
                .collect::<Vec<RetrievedChunk>>(),
        };

        let case_start = Instant::now();
        let mut retrieved = Vec::<RetrievedChunk>::new();
        for _ in 0..iterations {
            retrieved = retrieval.retrieve(&case.query, case.max_chunks.max(1))?;
        }
        let case_elapsed = case_start.elapsed();
        let case_avg_latency_ms = (case_elapsed.as_secs_f64() * 1000.0) / (iterations as f64);
        latency_ms.push(case_avg_latency_ms);

        let terms = normalize_eval_terms(&case.required_terms, &case.query);
        if terms.is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' produced no required terms; add required_terms or a richer query",
                case.id
            ));
        }

        let joined = retrieved
            .iter()
            .map(|chunk| chunk.text.to_ascii_lowercase())
            .collect::<Vec<String>>()
            .join("\n");

        let matched_terms = terms
            .iter()
            .filter(|term| joined.contains(term.as_str()))
            .count();
        let required_terms = terms.len();
        let min_term_matches = case
            .min_term_matches
            .unwrap_or(required_terms)
            .clamp(1, required_terms);
        let passed = matched_terms >= min_term_matches;
        if passed {
            passed_cases += 1;
        }

        case_reports.push(EvalCaseReport {
            id: case.id.clone(),
            passed,
            required_terms,
            matched_terms,
            retrieved_chunks: retrieved.len(),
            top_score: retrieved
                .first()
                .map(|chunk| chunk.score)
                .unwrap_or_default(),
            avg_latency_ms: round_metric(case_avg_latency_ms),
        });
    }

    let total_cases = dataset.cases.len();
    let failed_cases = total_cases.saturating_sub(passed_cases);
    let pass_rate = if total_cases == 0 {
        0.0
    } else {
        passed_cases as f64 / total_cases as f64
    };

    let mut sorted_latencies = latency_ms.clone();
    sorted_latencies.sort_by(|a, b| a.total_cmp(b));
    let avg_latency_ms = if latency_ms.is_empty() {
        0.0
    } else {
        latency_ms.iter().sum::<f64>() / latency_ms.len() as f64
    };
    let p95_latency_ms = percentile(&sorted_latencies, 95.0);

    let suite_elapsed_secs = suite_start.elapsed().as_secs_f64();
    let throughput_qps = if suite_elapsed_secs <= 0.0 {
        0.0
    } else {
        (total_cases as f64 * iterations as f64) / suite_elapsed_secs
    };

    let passed_threshold = pass_rate >= fail_under.clamp(0.0, 1.0);
    Ok(EvalRunReport {
        generated_at_unix_ms: unix_ms_now(),
        dataset_name: dataset.name.clone(),
        dataset_version: dataset.version.clone(),
        dataset_description: dataset.description.clone(),
        benchmark_iterations: iterations,
        total_cases,
        passed_cases,
        failed_cases,
        pass_rate: round_metric(pass_rate),
        fail_under: round_metric(fail_under.clamp(0.0, 1.0)),
        passed_threshold,
        avg_latency_ms: round_metric(avg_latency_ms),
        p95_latency_ms: round_metric(p95_latency_ms),
        throughput_qps: round_metric(throughput_qps),
        case_reports,
    })
}

pub fn write_eval_report(path: &str, report: &EvalRunReport) -> Result<()> {
    let path_buf = PathBuf::from(path);
    if let Some(parent) = path_buf.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create eval report directory '{}'",
                parent.display()
            )
        })?;
    }

    let payload =
        serde_json::to_string_pretty(report).context("failed to serialize eval report to json")?;
    std::fs::write(&path_buf, payload)
        .with_context(|| format!("failed to write eval report to '{}'", path_buf.display()))
}

pub fn run_eval(
    dataset_path: Option<String>,
    output_path: Option<String>,
    benchmark_iterations: usize,
    fail_under: f64,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let dataset_path = dataset_path.unwrap_or_else(|| DEFAULT_EVAL_DATASET_PATH.to_string());
    let output_path = output_path.unwrap_or_else(|| DEFAULT_EVAL_OUTPUT_PATH.to_string());
    let dataset = load_eval_dataset(&dataset_path)?;
    let report = run_eval_harness(&dataset, benchmark_iterations, fail_under)?;

    write_eval_report(&output_path, &report)?;
    telemetry.emit(
        "eval.completed",
        json!({
            "dataset": report.dataset_name,
            "dataset_version": report.dataset_version,
            "total_cases": report.total_cases,
            "pass_rate": report.pass_rate,
            "passed_threshold": report.passed_threshold,
            "output_path": output_path
        }),
    );

    println!(
        "Eval completed: dataset={} version={} cases={} pass_rate={:.3} threshold={:.3}",
        report.dataset_name,
        report.dataset_version,
        report.total_cases,
        report.pass_rate,
        report.fail_under
    );
    println!(
        "Benchmark: avg_latency_ms={:.3} p95_latency_ms={:.3} throughput_qps={:.3}",
        report.avg_latency_ms, report.p95_latency_ms, report.throughput_qps
    );
    println!("Report written to {}", output_path);

    if !report.passed_threshold {
        return Err(anyhow::anyhow!(
            "eval pass rate {:.3} is below threshold {:.3}",
            report.pass_rate,
            report.fail_under
        ));
    }

    Ok(())
}
