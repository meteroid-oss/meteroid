-- Add EndTrial event type for handling paid trial end via scheduled events
-- This allows paid trials to use normal billing cycles while having trial end
-- handled independently at the correct time (start_date + trial_duration)

ALTER TYPE "ScheduledEventTypeEnum" ADD VALUE IF NOT EXISTS 'END_TRIAL';
