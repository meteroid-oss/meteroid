SELECT pgmq.unbind_topic('#', 'workflow_engine');
SELECT pgmq.drop_queue('workflow_engine');

DROP TABLE workflow_event_sample;
DROP TABLE workflow_step_run;
DROP TABLE workflow_run;
ALTER TABLE workflow DROP CONSTRAINT workflow_active_version_fkey;
DROP TABLE workflow_version;
DROP TABLE workflow;

DROP TYPE "WorkflowStepStatusEnum";
DROP TYPE "WorkflowRunStatusEnum";
DROP TYPE "WorkflowTriggerTypeEnum";
