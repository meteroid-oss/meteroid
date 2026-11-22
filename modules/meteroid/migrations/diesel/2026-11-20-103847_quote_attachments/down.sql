CREATE FUNCTION pg_temp.attachment_ids(files JSONB) RETURNS UUID[] LANGUAGE sql IMMUTABLE AS $$
    SELECT coalesce(array_agg((f ->> 'id')::uuid), '{}') FROM jsonb_array_elements(files) f
$$;

ALTER TABLE quote ALTER COLUMN attachments DROP DEFAULT;
ALTER TABLE quote ALTER COLUMN attachments TYPE UUID[] USING pg_temp.attachment_ids(attachments);
ALTER TABLE quote ALTER COLUMN attachments SET DEFAULT '{}';

DROP TABLE quote_attachment;
