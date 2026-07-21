ALTER TABLE game_sessions
    ALTER COLUMN game_release_id DROP NOT NULL,
    ADD COLUMN source_revision BIGINT,
    ADD COLUMN execution_plan JSONB,
    ADD COLUMN is_preview BOOLEAN NOT NULL DEFAULT FALSE,
    ADD CONSTRAINT game_sessions_execution_source_check CHECK (
        (
            is_preview = FALSE
            AND game_release_id IS NOT NULL
            AND source_revision IS NULL
            AND execution_plan IS NULL
        )
        OR
        (
            is_preview = TRUE
            AND game_release_id IS NULL
            AND source_revision > 0
            AND jsonb_typeof(execution_plan) = 'object'
        )
    );

CREATE INDEX game_sessions_preview_created_idx
    ON game_sessions (project_id, created_at DESC)
    WHERE is_preview = TRUE;
