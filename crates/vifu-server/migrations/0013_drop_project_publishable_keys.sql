ALTER TABLE projects
    DROP COLUMN IF EXISTS publishable_key_prefix,
    DROP COLUMN IF EXISTS publishable_key_hash;
