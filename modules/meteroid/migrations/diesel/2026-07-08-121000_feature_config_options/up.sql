-- Allowed values for a "select"/enum Config feature. NULL = free-form config value
-- (number/boolean/text/json); a non-null array constrains the value to these options.
ALTER TABLE feature ADD COLUMN config_options TEXT[];
