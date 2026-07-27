ALTER TABLE projects
ADD COLUMN owner_user_id TEXT;

CREATE INDEX projects_owner_user_id_created_at_idx
ON projects (owner_user_id, created_at);
