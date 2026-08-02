CREATE TABLE comparisons (
    id BLOB PRIMARY KEY,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    deployment_id BLOB NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed', 'failed')),
    recommendation TEXT,
    not_exhaustive INTEGER NOT NULL CHECK (not_exhaustive IN (0, 1)),
    sequential_replay INTEGER NOT NULL CHECK (sequential_replay IN (0, 1)),
    corpus_agents INTEGER NOT NULL CHECK (corpus_agents >= 0),
    configured_models INTEGER NOT NULL CHECK (configured_models >= 0),
    tested_models INTEGER NOT NULL CHECK (tested_models >= 0),
    passed_models INTEGER NOT NULL CHECK (passed_models >= 0),
    device_architecture TEXT NOT NULL,
    device_backend TEXT,
    device_os TEXT,
    monotonic_duration_ms INTEGER NOT NULL CHECK (monotonic_duration_ms >= 0),
    started_at TEXT NOT NULL,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (completed_at IS NULL OR completed_at >= started_at)
);

CREATE INDEX comparisons_project_started_idx
    ON comparisons(project_id, started_at DESC);

CREATE TABLE comparison_runs (
    id BLOB PRIMARY KEY,
    comparison_id BLOB NOT NULL REFERENCES comparisons(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    combination_id TEXT NOT NULL,
    label TEXT NOT NULL,
    rule TEXT NOT NULL,
    routes TEXT NOT NULL CHECK (json_valid(routes)),
    route_labels TEXT NOT NULL CHECK (json_valid(route_labels)),
    outcome TEXT NOT NULL CHECK (outcome IN ('passed', 'failed')),
    first_total_ms INTEGER,
    first_run_cold INTEGER CHECK (first_run_cold IS NULL OR first_run_cold IN (0, 1)),
    repeat_runs_resident INTEGER
        CHECK (repeat_runs_resident IS NULL OR repeat_runs_resident IN (0, 1)),
    repeat_total_median_ms INTEGER,
    repeat_total_min_ms INTEGER,
    repeat_total_max_ms INTEGER,
    repeat_total_samples INTEGER,
    repeat_ttft_median_ms INTEGER,
    repeat_ttft_min_ms INTEGER,
    repeat_ttft_max_ms INTEGER,
    repeat_ttft_samples INTEGER,
    tokens_per_second REAL,
    first_process_cpu_percent REAL,
    process_cpu_percent REAL,
    peak_rss_bytes INTEGER,
    error TEXT,
    UNIQUE (comparison_id, position),
    UNIQUE (comparison_id, combination_id),
    CHECK (first_total_ms IS NULL OR first_total_ms >= 0),
    CHECK (
        (repeat_total_median_ms IS NULL AND repeat_total_min_ms IS NULL
            AND repeat_total_max_ms IS NULL AND repeat_total_samples IS NULL)
        OR
        (repeat_total_median_ms IS NOT NULL AND repeat_total_min_ms IS NOT NULL
            AND repeat_total_max_ms IS NOT NULL AND repeat_total_samples IS NOT NULL
            AND repeat_total_samples > 0
            AND repeat_total_min_ms <= repeat_total_median_ms
            AND repeat_total_median_ms <= repeat_total_max_ms)
    ),
    CHECK (
        (repeat_ttft_median_ms IS NULL AND repeat_ttft_min_ms IS NULL
            AND repeat_ttft_max_ms IS NULL AND repeat_ttft_samples IS NULL)
        OR
        (repeat_ttft_median_ms IS NOT NULL AND repeat_ttft_min_ms IS NOT NULL
            AND repeat_ttft_max_ms IS NOT NULL AND repeat_ttft_samples IS NOT NULL
            AND repeat_ttft_samples > 0
            AND repeat_ttft_min_ms <= repeat_ttft_median_ms
            AND repeat_ttft_median_ms <= repeat_ttft_max_ms)
    ),
    CHECK (
        outcome <> 'passed'
        OR (first_total_ms IS NOT NULL AND repeat_total_samples = 3 AND error IS NULL)
    ),
    CHECK (outcome <> 'failed' OR error IS NOT NULL)
);

CREATE INDEX comparison_runs_comparison_position_idx
    ON comparison_runs(comparison_id, position ASC);
