CREATE TABLE runtime_deployment_apply_states (
    deployment_id UUID NOT NULL REFERENCES runtime_deployments(id) ON DELETE CASCADE,
    gateway_id TEXT NOT NULL,
    release_version BIGINT NOT NULL CHECK (release_version > 0),
    content_hash TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (deployment_id, gateway_id)
);

CREATE INDEX runtime_deployment_apply_states_gateway_idx
    ON runtime_deployment_apply_states(gateway_id, applied_at DESC);
