-- @generated - DO NOT EDIT 
-- CreateEnum
CREATE TYPE "BillingProviderEnum" AS ENUM ('MANUAL', 'STRIPE');

-- CreateEnum
CREATE TYPE "InvoiceExternalStatusEnum" AS ENUM ('DELETED', 'DRAFT', 'FINALIZED', 'PAID', 'PAYMENT_FAILED', 'UNCOLLECTIBLE', 'VOID');

-- CreateEnum
CREATE TYPE "InvoiceStatusEnum" AS ENUM ('DRAFT', 'FINALIZED', 'PENDING_FINALIZATION', 'VOID');

-- CreateEnum
CREATE TYPE "InvoiceScheduleTypeEnum" AS ENUM ('ADVANCE', 'ARREARS', 'CREDIT_PURCHASE', 'TRIAL');

-- CreateTable
CREATE TABLE "invoice" (
    "id" UUID NOT NULL,
    "status" "InvoiceStatusEnum" NOT NULL DEFAULT 'DRAFT',
    "schedule_type" "InvoiceScheduleTypeEnum" NOT NULL,
    "billing_provider" "BillingProviderEnum" NOT NULL,
    "external_status" "InvoiceExternalStatusEnum",
    "start_date" TIMESTAMP(3) NOT NULL,
    "end_date" TIMESTAMP(3) NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3),
    "tenant_id" UUID NOT NULL,
    "customer_id" UUID NOT NULL,
    "subscription_id" UUID NOT NULL,
    "currency" TEXT NOT NULL,
    "days_until_due" INTEGER,
    "lines" JSONB NOT NULL,
    "billing_config" JSONB,

    CONSTRAINT "invoice_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "webhook" (
    "id" UUID NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "endpoint_uid" TEXT NOT NULL,
    "tenant_id" UUID NOT NULL,
    "provider" TEXT NOT NULL,
    "enabled" BOOLEAN NOT NULL DEFAULT true,
    "security" JSONB,

    CONSTRAINT "webhook_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "webhook_event" (
    "id" UUID NOT NULL,
    "received_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "action" TEXT,
    "key" TEXT NOT NULL,
    "processed" BOOLEAN NOT NULL DEFAULT false,
    "attempts" INTEGER NOT NULL DEFAULT 0,
    "error" TEXT,
    "webhook_id" UUID NOT NULL,

    CONSTRAINT "webhook_event_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "webhook_endpoint_uid_key" ON "webhook"("endpoint_uid");

-- CreateIndex
CREATE INDEX "webhook_endpoint_uid_provider" ON "webhook"("endpoint_uid", "provider");

-- AddForeignKey
ALTER TABLE "webhook_event" ADD CONSTRAINT "webhook_event_webhook_id_fkey" FOREIGN KEY ("webhook_id") REFERENCES "webhook"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
