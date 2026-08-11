ALTER TABLE projects ADD COLUMN app_id TEXT;

UPDATE projects
SET app_id = 'vifu_app_'
    || CASE WHEN typeof(id) = 'blob' THEN lower(hex(id)) ELSE replace(lower(id), '-', '') END
    || CASE WHEN typeof(id) = 'blob' THEN lower(hex(id)) ELSE replace(lower(id), '-', '') END
WHERE app_id IS NULL;

CREATE UNIQUE INDEX projects_app_id_unique ON projects(app_id);

INSERT OR IGNORE INTO runtime_distributions(
    id,
    project_id,
    deployment_id,
    name,
    public_id,
    status,
    max_gateways
)
SELECT
    randomblob(16),
    project.id,
    deployment.id,
    'App registration',
    project.app_id,
    'active',
    1000
FROM projects AS project
JOIN runtime_deployments AS deployment
  ON deployment.project_id = project.id
 AND deployment.is_primary = 1;
