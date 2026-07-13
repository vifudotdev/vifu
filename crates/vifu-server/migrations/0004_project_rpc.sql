DROP INDEX IF EXISTS projects_gateway_service_idx;

ALTER TABLE projects DROP COLUMN service_id;

ALTER TABLE endpoint_traces
    ALTER COLUMN endpoint_id DROP NOT NULL,
    ADD COLUMN project_id UUID REFERENCES projects(id) ON DELETE CASCADE;

ALTER TABLE endpoint_traces
    ADD CONSTRAINT endpoint_traces_single_resource_check
    CHECK ((endpoint_id IS NOT NULL) <> (project_id IS NOT NULL));

CREATE INDEX endpoint_traces_project_created_idx
    ON endpoint_traces (project_id, created_at DESC)
    WHERE project_id IS NOT NULL;
