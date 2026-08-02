CREATE TABLE comparisons (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    deployment_id UUID NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed', 'failed')),
    recommendation TEXT,
    not_exhaustive BOOLEAN NOT NULL,
    sequential_replay BOOLEAN NOT NULL,
    corpus_agents INTEGER NOT NULL CHECK (corpus_agents >= 0),
    configured_models INTEGER NOT NULL CHECK (configured_models >= 0),
    tested_models INTEGER NOT NULL CHECK (tested_models >= 0),
    passed_models INTEGER NOT NULL CHECK (passed_models >= 0),
    device_architecture TEXT NOT NULL,
    device_backend TEXT,
    device_os TEXT,
    monotonic_duration_ms BIGINT NOT NULL CHECK (monotonic_duration_ms >= 0),
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (completed_at IS NULL OR completed_at >= started_at)
);

CREATE INDEX comparisons_project_started_idx
    ON comparisons(project_id, started_at DESC);

CREATE TABLE comparison_runs (
    id UUID PRIMARY KEY,
    comparison_id UUID NOT NULL REFERENCES comparisons(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    combination_id TEXT NOT NULL,
    label TEXT NOT NULL,
    rule TEXT NOT NULL,
    routes JSONB NOT NULL,
    route_labels JSONB NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('passed', 'failed')),
    first_total_ms BIGINT,
    first_run_cold BOOLEAN,
    repeat_runs_resident BOOLEAN,
    repeat_total_median_ms BIGINT,
    repeat_total_min_ms BIGINT,
    repeat_total_max_ms BIGINT,
    repeat_total_samples INTEGER,
    repeat_ttft_median_ms BIGINT,
    repeat_ttft_min_ms BIGINT,
    repeat_ttft_max_ms BIGINT,
    repeat_ttft_samples INTEGER,
    tokens_per_second DOUBLE PRECISION,
    first_process_cpu_percent DOUBLE PRECISION,
    process_cpu_percent DOUBLE PRECISION,
    peak_rss_bytes BIGINT,
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
