CREATE TABLE project_canvas_nodes (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    position JSONB NOT NULL DEFAULT '{}'::jsonb,
    profile_id UUID REFERENCES agent_profiles(id) ON DELETE SET NULL,
    binding_id UUID REFERENCES agent_bindings(id) ON DELETE SET NULL,
    gateway_id TEXT,
    resource_id TEXT,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    inputs JSONB NOT NULL DEFAULT '{}'::jsonb,
    outputs JSONB NOT NULL DEFAULT '{}'::jsonb,
    exposed BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX project_canvas_nodes_project_idx
    ON project_canvas_nodes (project_id, created_at ASC);

CREATE UNIQUE INDEX project_canvas_nodes_project_binding_idx
    ON project_canvas_nodes (project_id, binding_id)
    WHERE binding_id IS NOT NULL;

CREATE TABLE project_canvas_edges (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_node_id UUID NOT NULL REFERENCES project_canvas_nodes(id) ON DELETE CASCADE,
    source_handle TEXT,
    target_node_id UUID NOT NULL REFERENCES project_canvas_nodes(id) ON DELETE CASCADE,
    target_handle TEXT,
    kind TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX project_canvas_edges_project_idx
    ON project_canvas_edges (project_id, created_at ASC);
