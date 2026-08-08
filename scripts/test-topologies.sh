#!/bin/sh
set -eu

repository_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
report_root="${VIFU_TOPOLOGY_REPORT_DIR:-$repository_root/target/topology-live}"
run_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
run_dir="$report_root/run-$run_stamp-$$"
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-topology-live.XXXXXX")"
results_file="$state_dir/results.tsv"
started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
original_home="${HOME:-}"
cargo_home="${CARGO_HOME:-$original_home/.cargo}"
rustup_home="${RUSTUP_HOME:-$original_home/.rustup}"
cargo_target_dir="${CARGO_TARGET_DIR:-$repository_root/target}"
failures=0

cleanup() {
  rm -rf -- "$state_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$run_dir/logs"
: > "$results_file"
cd "$repository_root"

run_case() {
  name="$1"
  package="$2"
  test_filter="$3"
  exact="$4"
  case_root="$state_dir/$name"
  case_home="$case_root/home"
  case_tmp="$case_root/tmp"
  log_file="$run_dir/logs/$name.log"
  mkdir -p "$case_home" "$case_tmp"

  printf '%-36s' "$name"
  started_seconds="$(date +%s)"
  if [ "$exact" = "1" ]; then
    if env -i \
      HOME="$case_home" \
      PATH="$PATH" \
      TMPDIR="$case_tmp" \
      CARGO_HOME="$cargo_home" \
      RUSTUP_HOME="$rustup_home" \
      CARGO_TARGET_DIR="$cargo_target_dir" \
      CARGO_INCREMENTAL=0 \
      CARGO_TERM_COLOR=never \
      RUST_BACKTRACE=1 \
      cargo test --locked -p "$package" "$test_filter" -- \
        --exact --nocapture --test-threads=1 >"$log_file" 2>&1; then
      status=0
    else
      status=$?
    fi
  else
    if env -i \
      HOME="$case_home" \
      PATH="$PATH" \
      TMPDIR="$case_tmp" \
      CARGO_HOME="$cargo_home" \
      RUSTUP_HOME="$rustup_home" \
      CARGO_TARGET_DIR="$cargo_target_dir" \
      CARGO_INCREMENTAL=0 \
      CARGO_TERM_COLOR=never \
      RUST_BACKTRACE=1 \
      cargo test --locked -p "$package" "$test_filter" -- \
        --nocapture --test-threads=1 >"$log_file" 2>&1; then
      status=0
    else
      status=$?
    fi
  fi
  finished_seconds="$(date +%s)"
  duration_seconds=$((finished_seconds - started_seconds))
  if [ "$status" -eq 0 ]; then
    result="passed"
    printf 'PASS  %ss\n' "$duration_seconds"
  else
    result="failed"
    failures=$((failures + 1))
    printf 'FAIL  %ss\n' "$duration_seconds"
    tail -n 30 "$log_file" >&2
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$name" "$package" "$test_filter" "$result" "$duration_seconds" \
    "logs/$name.log" >> "$results_file"
}

run_case \
  gateway-server-monitor-reconnect \
  vifu-server \
  monitor::tests::live_gateway_server_monitor_reconnects_with_persisted_session \
  1
run_case \
  project-monitor-isolation \
  vifu-server \
  monitor::tests::live_project_monitor_scope_filters_shared_gateway \
  1
run_case \
  project-monitor-multi-device \
  vifu-server \
  monitor::tests::project_monitor_allows_every_gateway_assigned_to_the_deployment \
  1
run_case \
  project-monitor-resource-routing \
  vifu-server \
  monitor::tests::project_monitor_filters_by_physical_resource_instead_of_profile_slug \
  1
run_case \
  agent-invocation-wire \
  vifu-server \
  websocket::tests::agent_gateway_websocket_uses_frame_transport_for_invocations \
  1
run_case \
  guest-bootstrap \
  vifu-server \
  tests::guest_gateway_bootstrap_is_idempotent_and_claimable \
  1
run_case \
  enrollment-one-time \
  vifu-server \
  db::tests::sqlite_consumes_gateway_enrollment_once_and_assigns_the_project \
  1
run_case \
  enrollment-concurrency \
  vifu-server \
  db::tests::sqlite_gateway_enrollment_is_atomic_under_concurrency \
  1
run_case \
  distribution-installation \
  vifu-server \
  websocket::tests::runtime_distribution_authorizes_a_new_installation_without_guest_bootstrap \
  1
run_case \
  distribution-device-limit \
  vifu-server \
  db::tests::sqlite_runtime_distribution_is_idempotent_and_enforces_its_device_limit \
  1
run_case \
  gateway-telemetry-retry \
  vifu-gateway \
  relay::tests::telemetry_worker_drains_burst_and_retries_without_a_new_invocation \
  1
run_case \
  embedded-monitor-content-consent \
  vifu-gateway \
  embedded::tests::embedded_monitor_io_requires_explicit_consent \
  1
run_case \
  cli-topology-selection \
  vifu \
  launcher::tests:: \
  0
run_case \
  cli-local-private-device-ingress \
  vifu \
  runtime_config::tests::all_interface_server_address_enables_managed_tls_and_guest_device_enrollment \
  1
run_case \
  cli-pairing-correlation \
  vifu \
  tui::model::tests::pairing_closes_only_for_its_enrollment \
  1

finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
node scripts/topology-live-report.mjs \
  "$results_file" \
  "$run_dir/report.json" \
  "$run_dir/junit.xml" \
  "$started_at" \
  "$finished_at"

printf '\nReports:\n  %s\n  %s\n' "$run_dir/report.json" "$run_dir/junit.xml"
if [ "$failures" -ne 0 ]; then
  printf '%s\n' "$failures topology live-test case(s) failed." >&2
  exit 1
fi
printf '%s\n' "All topology live-test cases passed."
