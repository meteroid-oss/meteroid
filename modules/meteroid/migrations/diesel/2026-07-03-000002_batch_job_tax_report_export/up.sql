-- New batch job kind: the scalable, chunked tax-report CSV export.
ALTER TYPE "BatchJobTypeEnum" ADD VALUE IF NOT EXISTS 'TAX_REPORT_EXPORT';
