#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

REQUESTS="${PERF_REQUESTS:-300}"
CONCURRENCY="${PERF_CONCURRENCY:-30}"
HOST="${PERF_HOST:-127.0.0.1}"
PORT="${PERF_PORT:-8789}"
HEALTH_P95_TARGET_MS="${PERF_HEALTH_P95_TARGET_MS:-50}"
A2A_P95_TARGET_MS="${PERF_A2A_P95_TARGET_MS:-80}"
EVAL_P95_TARGET_MS="${PERF_EVAL_P95_TARGET_MS:-5}"
SERVER_RSS_TARGET_MB="${PERF_SERVER_RSS_TARGET_MB:-300}"
EVAL_ITERATIONS="${PERF_EVAL_BENCH_ITERATIONS:-300}"
EVAL_FAIL_UNDER="${PERF_EVAL_FAIL_UNDER:-0.90}"
EVAL_OUTPUT="${PERF_EVAL_OUTPUT:-.zavora/evals/perf-reliability.json}"
OUTPUT_PATH="${PERF_OUTPUT:-.zavora/perf/latest.json}"

mkdir -p "$(dirname "${EVAL_OUTPUT}")" "$(dirname "${OUTPUT_PATH}")"

if ! command -v jq >/dev/null 2>&1; then
  echo "[PERF] jq is required for summary/report parsing." >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "[PERF] curl is required for endpoint stress tests." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
SERVER_LOG="${TMP_DIR}/server.log"
SERVER_PID=""

cleanup() {
  if [ -n "${SERVER_PID}" ] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill "${SERVER_PID}" >/dev/null 2>&1 || true
    wait "${SERVER_PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT INT TERM

float_le() {
  awk -v left="$1" -v right="$2" 'BEGIN { exit (left <= right ? 0 : 1) }'
}

run_http_load() {
  local name="$1"
  local url="$2"
  local method="$3"
  local body="$4"
  local expected="$5"
  local raw="${TMP_DIR}/${name}.csv"

  export LOAD_URL="${url}" LOAD_METHOD="${method}" LOAD_BODY="${body}"
  seq "${REQUESTS}" | xargs -P "${CONCURRENCY}" -I{} bash -c '
    if [ -n "${LOAD_BODY}" ]; then
      curl -sS -o /dev/null -w "%{http_code},%{time_total}\n" \
        -H "Content-Type: application/json" \
        -X "${LOAD_METHOD}" \
        --data "${LOAD_BODY}" \
        "${LOAD_URL}"
    else
      curl -sS -o /dev/null -w "%{http_code},%{time_total}\n" \
        -X "${LOAD_METHOD}" \
        "${LOAD_URL}"
    fi
  ' > "${raw}"

  local bad_count
  bad_count="$(
    awk -F, -v expected="${expected}" '$1 != expected { bad += 1 } END { print bad + 0 }' "${raw}"
  )"
  if [ "${bad_count}" -ne 0 ]; then
    echo "[PERF] ${name} produced ${bad_count} non-${expected} responses." >&2
    return 1
  fi

  cut -d, -f2 "${raw}" | sort -n | awk '
    {
      sample[NR] = $1;
      sum += $1;
    }
    END {
      if (NR == 0) {
        print "0,0.000,0.000,0.000,0.000,0.000";
        exit;
      }
      p95 = int((NR * 95 + 99) / 100);
      if (p95 < 1) p95 = 1;
      if (p95 > NR) p95 = NR;
      p99 = int((NR * 99 + 99) / 100);
      if (p99 < 1) p99 = 1;
      if (p99 > NR) p99 = NR;
      avg = sum / NR;
      printf "%d,%.3f,%.3f,%.3f,%.3f,%.3f\n", NR, avg * 1000, sample[p95] * 1000, sample[p99] * 1000, sample[NR] * 1000, sample[1] * 1000;
    }
  '
}

echo "[PERF] Running eval benchmark baseline..."
cargo run -- eval run \
  --dataset evals/datasets/retrieval-baseline.v1.json \
  --output "${EVAL_OUTPUT}" \
  --benchmark-iterations "${EVAL_ITERATIONS}" \
  --fail-under "${EVAL_FAIL_UNDER}"

eval_avg_ms="$(jq -r '.avg_latency_ms' "${EVAL_OUTPUT}")"
eval_p95_ms="$(jq -r '.p95_latency_ms' "${EVAL_OUTPUT}")"
eval_qps="$(jq -r '.throughput_qps' "${EVAL_OUTPUT}")"
eval_pass_rate="$(jq -r '.pass_rate' "${EVAL_OUTPUT}")"

echo "[PERF] Starting server for endpoint stress tests..."
cargo run -- server serve --host "${HOST}" --port "${PORT}" > "${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

ready=0
for _ in $(seq 1 80); do
  if curl -fsS "http://${HOST}:${PORT}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.25
done

if [ "${ready}" -ne 1 ]; then
  echo "[PERF] Server failed to become ready." >&2
  cat "${SERVER_LOG}" >&2 || true
  exit 1
fi

echo "[PERF] Running /healthz load (${REQUESTS} requests @ concurrency ${CONCURRENCY})..."
IFS=, read -r health_count health_avg_ms health_p95_ms health_p99_ms health_max_ms health_min_ms <<EOF
$(run_http_load "healthz" "http://${HOST}:${PORT}/healthz" "GET" "" "200")
EOF

payload='{"from_agent":"perf-client","to_agent":"zavora-router","message_id":"perf-001","correlation_id":"run-001","payload":{"intent":"load-check"}}'

echo "[PERF] Running /v1/a2a/ping load (${REQUESTS} requests @ concurrency ${CONCURRENCY})..."
IFS=, read -r a2a_count a2a_avg_ms a2a_p95_ms a2a_p99_ms a2a_max_ms a2a_min_ms <<EOF
$(run_http_load "a2a_ping" "http://${HOST}:${PORT}/v1/a2a/ping" "POST" "${payload}" "200")
EOF

server_rss_kb="$(ps -o rss= -p "${SERVER_PID}" | tr -d '[:space:]')"
server_rss_kb="${server_rss_kb:-0}"
server_rss_mb="$(awk -v kb="${server_rss_kb}" 'BEGIN { printf "%.2f", kb / 1024.0 }')"

failures=()
if ! float_le "${eval_p95_ms}" "${EVAL_P95_TARGET_MS}"; then
  failures+=("eval p95 (${eval_p95_ms} ms) exceeded target (${EVAL_P95_TARGET_MS} ms)")
fi
if ! float_le "${health_p95_ms}" "${HEALTH_P95_TARGET_MS}"; then
  failures+=("/healthz p95 (${health_p95_ms} ms) exceeded target (${HEALTH_P95_TARGET_MS} ms)")
fi
if ! float_le "${a2a_p95_ms}" "${A2A_P95_TARGET_MS}"; then
  failures+=("/v1/a2a/ping p95 (${a2a_p95_ms} ms) exceeded target (${A2A_P95_TARGET_MS} ms)")
fi
if ! float_le "${server_rss_mb}" "${SERVER_RSS_TARGET_MB}"; then
  failures+=("server RSS (${server_rss_mb} MB) exceeded target (${SERVER_RSS_TARGET_MB} MB)")
fi

failure_json="$(printf '%s\n' "${failures[@]-}" | jq -R . | jq -s 'map(select(length > 0))')"
passed=true
if [ "${#failures[@]}" -gt 0 ]; then
  passed=false
fi

timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

jq -n \
  --arg generated_at "${timestamp}" \
  --arg host "${HOST}" \
  --argjson port "${PORT}" \
  --argjson requests "${REQUESTS}" \
  --argjson concurrency "${CONCURRENCY}" \
  --arg eval_report "${EVAL_OUTPUT}" \
  --argjson eval_avg_ms "${eval_avg_ms}" \
  --argjson eval_p95_ms "${eval_p95_ms}" \
  --argjson eval_throughput_qps "${eval_qps}" \
  --argjson eval_pass_rate "${eval_pass_rate}" \
  --argjson health_count "${health_count}" \
  --argjson health_avg_ms "${health_avg_ms}" \
  --argjson health_p95_ms "${health_p95_ms}" \
  --argjson health_p99_ms "${health_p99_ms}" \
  --argjson health_max_ms "${health_max_ms}" \
  --argjson health_min_ms "${health_min_ms}" \
  --argjson a2a_count "${a2a_count}" \
  --argjson a2a_avg_ms "${a2a_avg_ms}" \
  --argjson a2a_p95_ms "${a2a_p95_ms}" \
  --argjson a2a_p99_ms "${a2a_p99_ms}" \
  --argjson a2a_max_ms "${a2a_max_ms}" \
  --argjson a2a_min_ms "${a2a_min_ms}" \
  --argjson server_rss_mb "${server_rss_mb}" \
  --argjson target_eval_p95_ms "${EVAL_P95_TARGET_MS}" \
  --argjson target_health_p95_ms "${HEALTH_P95_TARGET_MS}" \
  --argjson target_a2a_p95_ms "${A2A_P95_TARGET_MS}" \
  --argjson target_server_rss_mb "${SERVER_RSS_TARGET_MB}" \
  --argjson passed "${passed}" \
  --argjson failures "${failure_json}" \
  '{
    generated_at: $generated_at,
    config: {
      host: $host,
      port: $port,
      requests_per_endpoint: $requests,
      concurrency: $concurrency
    },
    targets: {
      eval_p95_ms: $target_eval_p95_ms,
      health_p95_ms: $target_health_p95_ms,
      a2a_ping_p95_ms: $target_a2a_p95_ms,
      server_rss_mb: $target_server_rss_mb
    },
    eval: {
      report: $eval_report,
      pass_rate: $eval_pass_rate,
      avg_latency_ms: $eval_avg_ms,
      p95_latency_ms: $eval_p95_ms,
      throughput_qps: $eval_throughput_qps
    },
    endpoint_load: {
      healthz: {
        sample_count: $health_count,
        avg_ms: $health_avg_ms,
        p95_ms: $health_p95_ms,
        p99_ms: $health_p99_ms,
        min_ms: $health_min_ms,
        max_ms: $health_max_ms
      },
      a2a_ping: {
        sample_count: $a2a_count,
        avg_ms: $a2a_avg_ms,
        p95_ms: $a2a_p95_ms,
        p99_ms: $a2a_p99_ms,
        min_ms: $a2a_min_ms,
        max_ms: $a2a_max_ms
      }
    },
    resources: {
      server_rss_mb: $server_rss_mb
    },
    status: {
      passed: $passed,
      failures: $failures
    }
  }' > "${OUTPUT_PATH}"

echo "[PERF] Summary written to ${OUTPUT_PATH}"
echo "[PERF] eval p95=${eval_p95_ms}ms, healthz p95=${health_p95_ms}ms, a2a p95=${a2a_p95_ms}ms, rss=${server_rss_mb}MB"

if [ "${passed}" != "true" ]; then
  echo "[PERF] Target violations detected:" >&2
  printf '  - %s\n' "${failures[@]}" >&2
  exit 1
fi

echo "[PERF] Passed."
