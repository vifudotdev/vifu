ALTER TABLE projects ADD COLUMN app_id TEXT;

UPDATE projects
SET app_id = 'vifu_app_' || replace(id::text, '-', '') || replace(id::text, '-', '')
WHERE app_id IS NULL;

ALTER TABLE projects ALTER COLUMN app_id SET NOT NULL;
ALTER TABLE projects ADD CONSTRAINT projects_app_id_unique UNIQUE (app_id);

INSERT INTO runtime_distributions(
    id,
    project_id,
    deployment_id,
    name,
    public_id,
    status,
    max_gateways
)
SELECT
    gen_random_uuid(),
    project.id,
    deployment.id,
    'App registration',
    project.app_id,
    'active',
    1000
FROM projects AS project
JOIN runtime_deployments AS deployment
  ON deployment.project_id = project.id
 AND deployment.is_primary
ON CONFLICT (public_id) DO NOTHING;
