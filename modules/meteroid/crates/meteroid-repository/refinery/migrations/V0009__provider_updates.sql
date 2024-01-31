-- @generated - DO NOT EDIT 
/*
  Warnings:

  - The values [CREDIT_PURCHASE,TRIAL] on the enum `InvoiceScheduleTypeEnum` will be removed. If these variants are still used in the database, this will fail.
  - The values [PENDING_FINALIZATION] on the enum `InvoiceStatusEnum` will be removed. If these variants are still used in the database, this will fail.
  - You are about to drop the column `billing_config` on the `invoice` table. All the data in the column will be lost.
  - You are about to drop the column `billing_provider` on the `invoice` table. All the data in the column will be lost.
  - You are about to drop the column `lines` on the `invoice` table. All the data in the column will be lost.
  - You are about to drop the column `webhook_id` on the `webhook_event` table. All the data in the column will be lost.
  - You are about to drop the `webhook` table. If the table is not empty, all the data it contains will be lost.
  - A unique constraint covering the columns `[invoice_id]` on the table `invoice` will be added. If there are existing duplicate values, this will fail.
  - A unique constraint covering the columns `[external_invoice_id]` on the table `invoice` will be added. If there are existing duplicate values, this will fail.
  - Added the required column `grace_period_hours` to the `invoice` table without a default value. This is not possible if the table is not empty.
  - Added the required column `invoicing_provider` to the `invoice` table without a default value. This is not possible if the table is not empty.
  - Added the required column `line_items` to the `invoice` table without a default value. This is not possible if the table is not empty.
  - Added the required column `provider_config_id` to the `webhook_event` table without a default value. This is not possible if the table is not empty.

*/
-- CreateEnum
CREATE TYPE "InvoicingProviderEnum" AS ENUM ('STRIPE');

-- AlterEnum
BEGIN;
CREATE TYPE "InvoiceScheduleTypeEnum_new" AS ENUM ('ADVANCE', 'ARREARS');
ALTER TABLE "invoice" ALTER COLUMN "schedule_type" TYPE "InvoiceScheduleTypeEnum_new" USING ("schedule_type"::text::"InvoiceScheduleTypeEnum_new");
ALTER TYPE "InvoiceScheduleTypeEnum" RENAME TO "InvoiceScheduleTypeEnum_old";
ALTER TYPE "InvoiceScheduleTypeEnum_new" RENAME TO "InvoiceScheduleTypeEnum";
DROP TYPE "InvoiceScheduleTypeEnum_old";
COMMIT;

-- AlterEnum
BEGIN;
CREATE TYPE "InvoiceStatusEnum_new" AS ENUM ('DRAFT', 'FINALIZED', 'PENDING', 'VOID');
ALTER TABLE "invoice" ALTER COLUMN "status" DROP DEFAULT;
ALTER TABLE "invoice" ALTER COLUMN "status" TYPE "InvoiceStatusEnum_new" USING ("status"::text::"InvoiceStatusEnum_new");
ALTER TYPE "InvoiceStatusEnum" RENAME TO "InvoiceStatusEnum_old";
ALTER TYPE "InvoiceStatusEnum_new" RENAME TO "InvoiceStatusEnum";
DROP TYPE "InvoiceStatusEnum_old";
ALTER TABLE "invoice" ALTER COLUMN "status" SET DEFAULT 'DRAFT';
COMMIT;

-- DropForeignKey
ALTER TABLE "webhook_event" DROP CONSTRAINT "webhook_event_webhook_id_fkey";

-- AlterTable
ALTER TABLE "invoice" DROP COLUMN "billing_config",
DROP COLUMN "billing_provider",
DROP COLUMN "lines",
ADD COLUMN     "external_invoice_id" TEXT,
ADD COLUMN     "grace_period_hours" INTEGER NOT NULL,
ADD COLUMN     "invoice_id" TEXT,
ADD COLUMN     "invoicing_provider" "InvoicingProviderEnum" NOT NULL,
ADD COLUMN     "line_items" JSONB NOT NULL;

-- AlterTable
ALTER TABLE "webhook_event" DROP COLUMN "webhook_id",
ADD COLUMN     "provider_config_id" UUID NOT NULL;

-- DropTable
DROP TABLE "webhook";

-- DropEnum
DROP TYPE "BillingProviderEnum";

-- CreateTable
CREATE TABLE "provider_config" (
    "id" UUID NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "tenant_id" UUID NOT NULL,
    "wh_endpoint_uid" TEXT NOT NULL,
    "invoicing_provider" "InvoicingProviderEnum" NOT NULL,
    "enabled" BOOLEAN NOT NULL DEFAULT true,
    "webhook_security" JSONB NOT NULL,
    "api_security" JSONB NOT NULL,

    CONSTRAINT "provider_config_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "provider_config_wh_endpoint_uid_key" ON "provider_config"("wh_endpoint_uid");

-- CreateIndex
CREATE INDEX "ppc_endpoint_uid_provider" ON "provider_config"("wh_endpoint_uid", "invoicing_provider");

-- CreateIndex
CREATE UNIQUE INDEX "invoice_invoice_id_key" ON "invoice"("invoice_id");

-- CreateIndex
CREATE UNIQUE INDEX "invoice_external_invoice_id_key" ON "invoice"("external_invoice_id");

-- AddForeignKey
ALTER TABLE "webhook_event" ADD CONSTRAINT "webhook_event_provider_config_id_fkey" FOREIGN KEY ("provider_config_id") REFERENCES "provider_config"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
