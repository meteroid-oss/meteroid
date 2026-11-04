-- Add the forecast-breach alert condition kind (fires before a projected breach).
ALTER TYPE "AlertConditionKindEnum" ADD VALUE IF NOT EXISTS 'FORECAST';
