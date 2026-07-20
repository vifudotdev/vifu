WITH resolved AS (
    SELECT
        version.id AS version_id,
        COALESCE(
            NULLIF(binding.config->>'providerKey', ''),
            (
                SELECT NULLIF(session.metadata->>'providerId', '')
                FROM agent_gateway_sessions AS session
                WHERE session.gateway_id = binding.gateway_id
                  AND NULLIF(session.metadata->>'providerId', '') IS NOT NULL
                ORDER BY (session.status = 'connected') DESC, session.last_seen_at DESC
                LIMIT 1
            ),
            binding.provider
        ) AS provider_key
    FROM agent_profile_versions AS version
    JOIN LATERAL (
        SELECT candidate.*
        FROM agent_bindings AS candidate
        WHERE candidate.profile_id = version.profile_id
        ORDER BY candidate.created_at ASC
        LIMIT 1
    ) AS binding ON TRUE
)
UPDATE agent_profile_versions AS version
SET source = version.source || jsonb_build_object('providerKey', resolved.provider_key)
FROM resolved
WHERE resolved.version_id = version.id;

WITH resolved AS (
    SELECT
        capability.id AS capability_id,
        COALESCE(
            NULLIF(binding.config->>'providerKey', ''),
            (
                SELECT NULLIF(session.metadata->>'providerId', '')
                FROM agent_gateway_sessions AS session
                WHERE session.gateway_id = binding.gateway_id
                  AND NULLIF(session.metadata->>'providerId', '') IS NOT NULL
                ORDER BY (session.status = 'connected') DESC, session.last_seen_at DESC
                LIMIT 1
            ),
            binding.provider
        ) AS provider_key
    FROM agent_profile_capabilities AS capability
    JOIN agent_profile_versions AS version
        ON version.id = capability.profile_version_id
    JOIN LATERAL (
        SELECT candidate.*
        FROM agent_bindings AS candidate
        WHERE candidate.profile_id = version.profile_id
          AND candidate.provider = capability.provider_type
          AND candidate.agent_id = capability.resource_id
        ORDER BY candidate.created_at ASC
        LIMIT 1
    ) AS binding ON TRUE
)
UPDATE agent_profile_capabilities AS capability
SET provider_key = resolved.provider_key
FROM resolved
WHERE resolved.capability_id = capability.id;
