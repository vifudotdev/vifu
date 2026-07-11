UPDATE connector_sessions SET
    status = 'disconnected',
    last_seen_at = NOW(),
    disconnected_at = NOW()
WHERE status = 'connected';

CREATE UNIQUE INDEX connector_sessions_one_connected_idx
    ON connector_sessions (connector_id)
    WHERE status = 'connected';
