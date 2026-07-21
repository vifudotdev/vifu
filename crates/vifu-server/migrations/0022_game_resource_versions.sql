ALTER TABLE project_game_resources
    DROP CONSTRAINT project_game_resources_project_id_resource_key_key,
    ADD CONSTRAINT project_game_resources_project_key_version_key
        UNIQUE (project_id, resource_key, version);

ALTER TABLE game_asset_versions
    DROP CONSTRAINT game_asset_versions_project_id_content_hash_key,
    ADD CONSTRAINT game_asset_versions_project_asset_hash_key
        UNIQUE (project_id, asset_id, content_hash);
