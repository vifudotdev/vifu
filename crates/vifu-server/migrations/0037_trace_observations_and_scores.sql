ALTER TABLE trace_spans
    ADD COLUMN observation_type TEXT NOT NULL DEFAULT 'span'
        CHECK (observation_type IN ('span', 'generation', 'event')),
    ADD COLUMN model TEXT,
    ADD COLUMN model_parameters JSONB,
    ADD COLUMN completion_start_ms BIGINT,
    ADD COLUMN usage JSONB;

CREATE TABLE trace_scores (
    id UUID PRIMARY KEY,
    trace_id UUID NOT NULL REFERENCES endpoint_traces(id) ON DELETE CASCADE,
    span_id UUID REFERENCES trace_spans(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    data_type TEXT NOT NULL CHECK (data_type IN ('boolean', 'numeric', 'categorical')),
    value JSONB NOT NULL,
    source TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (trace_id, name, source)
);

CREATE INDEX trace_scores_trace_created_idx
    ON trace_scores (trace_id, created_at ASC);
