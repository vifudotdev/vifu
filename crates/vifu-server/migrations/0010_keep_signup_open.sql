UPDATE auth_settings
SET signup_enabled = TRUE, updated_at = NOW()
WHERE signup_enabled = FALSE;
