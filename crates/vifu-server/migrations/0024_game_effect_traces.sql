ALTER TABLE game_effects
    ADD COLUMN trace_id UUID REFERENCES endpoint_traces(id) ON DELETE SET NULL,
    ADD COLUMN parent_span_id UUID REFERENCES trace_spans(id) ON DELETE SET NULL,
    ADD CONSTRAINT game_effects_trace_context_check CHECK (
        (trace_id IS NULL AND parent_span_id IS NULL)
        OR (trace_id IS NOT NULL AND parent_span_id IS NOT NULL)
    );

CREATE INDEX game_effects_trace_idx
    ON game_effects (trace_id, created_at ASC)
    WHERE trace_id IS NOT NULL;
