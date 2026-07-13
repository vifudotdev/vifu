ALTER TABLE projects
    RENAME COLUMN access_key_prefix TO publishable_key_prefix;

ALTER TABLE projects
    RENAME COLUMN access_key_hash TO publishable_key_hash;
