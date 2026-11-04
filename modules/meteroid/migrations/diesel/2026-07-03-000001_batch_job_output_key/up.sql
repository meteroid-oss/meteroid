-- Generic success-artifact pointer for batch jobs (the counterpart to error_output_key,
-- which only ever holds the failed-rows CSV). The tax-report export writes its finished
-- combined CSV to object storage and records the document id here.
ALTER TABLE batch_job
    ADD COLUMN output_key TEXT NULL;
