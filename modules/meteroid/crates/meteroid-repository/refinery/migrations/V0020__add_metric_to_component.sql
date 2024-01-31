-- avoid requiring uuid-ossp, this function is native since pg13
-- Modify "fang_tasks" table
ALTER TABLE "fang_tasks"
    ALTER COLUMN "id" SET DEFAULT gen_random_uuid();
-- Modify "fang_tasks_archive" table
ALTER TABLE "fang_tasks_archive"
    ALTER COLUMN "id" SET DEFAULT gen_random_uuid();

-- Modify "price_component" table
ALTER TABLE "price_component"
    ADD COLUMN "billable_metric_id" uuid NULL,
    ADD CONSTRAINT "price_component_billable_metric_id_fkey" FOREIGN KEY ("billable_metric_id") REFERENCES "billable_metric" ("id") ON UPDATE CASCADE ON DELETE SET NULL;
