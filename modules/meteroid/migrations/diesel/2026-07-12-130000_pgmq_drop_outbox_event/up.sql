-- Part-2 cleanup: the dispatcher is gone and producers publish straight to topic
-- bindings, so the outbox_event queue (and its archive) is no longer used.
SELECT pgmq.drop_queue('outbox_event');
