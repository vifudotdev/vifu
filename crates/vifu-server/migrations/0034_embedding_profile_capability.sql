ALTER TABLE agent_profile_capabilities
    DROP CONSTRAINT agent_profile_capabilities_kind_check;

ALTER TABLE agent_profile_capabilities
    ADD CONSTRAINT agent_profile_capabilities_kind_check
    CHECK (kind IN ('chat', 'embedding', 'speech', 'transcription', 'realtime', 'tool'));
