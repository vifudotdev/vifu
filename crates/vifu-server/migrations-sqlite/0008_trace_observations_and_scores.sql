ALTER TABLE trace_spans
    ADD COLUMN observation_type TEXT NOT NULL DEFAULT 'span'
        CHECK (observation_type IN ('span', 'generation', 'event'));
ALTER TABLE trace_spans ADD COLUMN model TEXT;
ALTER TABLE trace_spans
    ADD COLUMN model_parameters TEXT
        CHECK (model_parameters IS NULL OR json_valid(model_parameters));
ALTER TABLE trace_spans ADD COLUMN completion_start_ms INTEGER;
ALTER TABLE trace_spans
    ADD COLUMN usage TEXT CHECK (usage IS NULL OR json_valid(usage));

CREATE TABLE trace_scores (
    id BLOB PRIMARY KEY,
    trace_id BLOB NOT NULL REFERENCES endpoint_traces(id) ON DELETE CASCADE,
    span_id BLOB REFERENCES trace_spans(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    data_type TEXT NOT NULL CHECK (data_type IN ('boolean', 'numeric', 'categorical')),
    value TEXT NOT NULL CHECK (json_valid(value)),
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (trace_id, name, source)
);

CREATE INDEX trace_scores_trace_created_idx
    ON trace_scores (trace_id, created_at ASC);
