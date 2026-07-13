UPDATE agent_gateway_sessions SET
    status = 'disconnected',
    last_seen_at = NOW(),
    disconnected_at = NOW()
WHERE status = 'connected';

CREATE UNIQUE INDEX agent_gateway_sessions_one_connected_idx
    ON agent_gateway_sessions (gateway_id)
    WHERE status = 'connected';
