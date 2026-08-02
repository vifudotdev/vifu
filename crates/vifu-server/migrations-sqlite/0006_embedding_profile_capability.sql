ALTER TABLE agent_profile_capabilities
    RENAME TO agent_profile_capabilities_before_embedding;

CREATE TABLE agent_profile_capabilities (
    id BLOB PRIMARY KEY,
    profile_version_id BLOB NOT NULL
        REFERENCES agent_profile_versions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('chat', 'embedding', 'speech', 'transcription', 'realtime', 'tool')),
    provider_type TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    resource_id TEXT,
    config TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config)),
    input_schema TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(input_schema)),
    output_schema TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(output_schema)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT INTO agent_profile_capabilities (
    id,
    profile_version_id,
    kind,
    provider_type,
    provider_key,
    resource_id,
    config,
    input_schema,
    output_schema,
    created_at
)
SELECT
    id,
    profile_version_id,
    kind,
    provider_type,
    provider_key,
    resource_id,
    config,
    input_schema,
    output_schema,
    created_at
FROM agent_profile_capabilities_before_embedding;

DROP TABLE agent_profile_capabilities_before_embedding;

CREATE INDEX agent_profile_capabilities_version_kind_idx
    ON agent_profile_capabilities (profile_version_id, kind, created_at ASC);
