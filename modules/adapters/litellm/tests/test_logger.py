import asyncio
from dataclasses import replace

import httpx

from meteroid_litellm.logger import MeteroidLogger

from conftest import Recorder, payload


def logger_for(settings, recorder):
    return MeteroidLogger(settings, transport=httpx.MockTransport(recorder))


async def log(logger, p):
    await logger.async_log_success_event({"standard_logging_object": p}, None, None, None)


async def test_events_are_batched_to_the_ingest_endpoint(settings):
    recorder = Recorder()
    logger = logger_for(replace(settings, batch_size=10), recorder)
    for i in range(2):
        await log(logger, payload(f"call-{i}"))
    assert recorder.bodies == [], "below the batch size nothing is sent yet"
    await log(logger, payload("call-2"))
    assert [len(b["events"]) for b in recorder.bodies] == [10, 2]
    request = recorder.requests[0]
    assert request.url.path == "/api/v2/events/ingest"
    assert request.headers["authorization"] == "Bearer pv_test_key"
    assert request.headers["user-agent"].startswith("meteroid-litellm/")
    assert recorder.bodies[0]["allow_partial_failures"] is True
    assert recorder.bodies[0]["allow_backfilling"] is True, "the events are over 23 hours old"
    assert not logger.queue
    await logger.aclose()


async def test_a_full_batch_is_sent_at_once(settings):
    recorder = Recorder()
    logger = logger_for(replace(settings, batch_size=4), recorder)
    await log(logger, payload("call-1"))
    assert len(recorder.bodies) == 1
    await logger.aclose()


async def test_a_batch_meteroid_cannot_take_is_sent_again_with_the_same_ids(settings):
    recorder = Recorder(statuses=[503, 429])
    logger = logger_for(settings, recorder)
    await log(logger, payload("call-1"))
    for _ in range(3):
        await logger.flush()
    assert len(recorder.bodies) == 3
    ids = [[e["event_id"] for e in body["events"]] for body in recorder.bodies]
    assert ids[0] == ids[1] == ids[2]
    assert not logger.queue
    await logger.aclose()


async def test_an_unreachable_meteroid_keeps_the_events(settings):
    def down(request):
        raise httpx.ConnectError("connection refused", request=request)

    logger = MeteroidLogger(settings, transport=httpx.MockTransport(down))
    await log(logger, payload("call-1"))
    await logger.flush()
    assert len(logger.queue) == 4
    await logger._client.aclose()


async def test_one_unknown_customer_never_drops_the_others(settings):
    recorder = Recorder()
    logger = logger_for(settings, recorder)
    known = payload("call-1")
    unknown = payload("call-2")
    unknown["metadata"]["user_api_key_customer_org_alias"] = "unknown"
    await log(logger, known)
    await log(logger, unknown)
    await logger.flush()
    assert logger.rejected == 4
    assert not logger.queue, "rejected events are not retried"
    await logger.aclose()


async def test_a_refused_batch_is_dropped_not_retried(settings):
    recorder = Recorder(statuses=[422])
    logger = logger_for(settings, recorder)
    await log(logger, payload("call-1"))
    await logger.flush()
    await logger.flush()
    assert len(recorder.bodies) == 1
    assert not logger.queue
    await logger.aclose()


async def test_the_queue_is_bounded(settings):
    logger = logger_for(replace(settings, max_queue=6, batch_size=500), Recorder())
    await log(logger, payload("call-1"))
    await log(logger, payload("call-2"))
    assert len(logger.queue) == 6
    assert logger.dropped == 2
    assert logger.queue[0]["event_id"] == "call-1:cache_read", "the oldest go first"
    logger._flusher.cancel()


async def test_the_periodic_flush_sends_what_is_queued(settings):
    recorder = Recorder()
    logger = logger_for(replace(settings, flush_interval=0.05), recorder)
    await log(logger, payload("call-1"))
    await asyncio.sleep(0.2)
    assert len(recorder.bodies) == 1
    await logger.aclose()


def test_synchronous_sdk_calls_are_sent_too(settings):
    recorder = Recorder()
    logger = MeteroidLogger(
        replace(settings, batch_size=4), sync_transport=httpx.MockTransport(recorder)
    )
    logger.log_success_event({"standard_logging_object": payload("call-1")}, None, None, None)
    assert len(recorder.bodies) == 1


async def test_a_full_queue_during_a_flush_never_drops_an_unsent_event(settings):
    sent = []

    def meteroid(request):
        body = __import__("json").loads(request.content)
        sent.extend(e["event_id"] for e in body["events"])
        if len(sent) == 4:
            for i in (2, 3, 4):
                logger.enqueue({"standard_logging_object": payload(f"call-{i}")})
        return httpx.Response(200, json={"accepted": len(body["events"]), "rejected": []})

    logger = MeteroidLogger(
        replace(settings, batch_size=4, max_queue=8), transport=httpx.MockTransport(meteroid)
    )
    logger.enqueue({"standard_logging_object": payload("call-1")})
    await logger.flush()
    assert sent[:4] == [f"call-1:{c}" for c in ("input", "output", "cache_read", "reasoning")]
    assert logger.dropped == 4, "the queue overflowed while the batch was out: call-2 went"
    unsent = [e["event_id"].split(":")[0] for e in logger.queue] + [i.split(":")[0] for i in sent[4:]]
    assert sorted(set(unsent)) == ["call-3", "call-4"]
    assert len(sent) + len(logger.queue) == 12, "every event kept is sent or still queued"
    await logger._client.aclose()


async def test_a_refused_key_keeps_the_batch_for_when_it_is_fixed(settings):
    recorder = Recorder(statuses=[401, 403])
    logger = logger_for(settings, recorder)
    await log(logger, payload("call-1"))
    await logger.flush()
    await logger.flush()
    assert len(logger.queue) == 4
    await logger.flush()
    assert len(recorder.bodies) == 3
    assert not logger.queue
    await logger.aclose()


def test_synchronous_calls_are_flushed_periodically_and_at_exit(settings):
    import time

    recorder = Recorder()
    logger = MeteroidLogger(
        replace(settings, batch_size=100, flush_interval=0.05),
        sync_transport=httpx.MockTransport(recorder),
    )
    logger.log_success_event({"standard_logging_object": payload("call-1")}, None, None, None)
    time.sleep(0.3)
    assert len(recorder.bodies) == 1, "a partial batch does not wait for a full one"

    logger.enqueue({"standard_logging_object": payload("call-2")})
    logger.flush_at_exit()
    assert len(recorder.bodies) == 2
    assert not logger.queue
