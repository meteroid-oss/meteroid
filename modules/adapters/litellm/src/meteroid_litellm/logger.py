"""The LiteLLM callback: queues each successful call's events and sends them in batches."""

from __future__ import annotations

import asyncio
import atexit
import logging
import threading
from collections import deque
from typing import Any, Deque, Dict, List, Optional

import httpx

from .client import MeteroidClient, RetryableError, SyncMeteroidClient
from .events import events_for
from .gate import CreditGate, pre_call_customer, refusal
from .settings import Settings

log = logging.getLogger("meteroid_litellm")

try:
    from litellm.integrations.custom_logger import CustomLogger as _Base
except Exception:  # LiteLLM is the host, not a dependency: tests run without it.

    class _Base:  # type: ignore[no-redef]
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            pass


class MeteroidLogger(_Base):
    def __init__(
        self,
        settings: Optional[Settings] = None,
        transport: Optional[httpx.AsyncBaseTransport] = None,
        sync_transport: Optional[httpx.BaseTransport] = None,
    ):
        super().__init__()
        self.settings = settings or Settings.from_env()
        self.queue: Deque[Dict[str, Any]] = deque()
        self.dropped = 0
        self.rejected = 0
        # Guards the queue: the proxy's event loop and the SDK's threads both add and take.
        self._lock = threading.Lock()
        self._client = MeteroidClient(self.settings, transport)
        self._sync_transport = sync_transport
        self._sync_client: Optional[SyncMeteroidClient] = None
        self._flusher: Optional[asyncio.Task] = None
        self._sync_flusher: Optional[threading.Thread] = None
        self._stopped = threading.Event()
        self.gate = CreditGate(self._client, self.settings) if self.settings.credit_gate else None
        if not self.settings.api_key:
            log.error("METEROID_API_KEY is not set: Meteroid refuses the events")
        atexit.register(self.flush_at_exit)

    def _bound(self) -> None:
        while len(self.queue) > self.settings.max_queue:
            self.queue.popleft()
            self.dropped += 1

    def enqueue(self, kwargs: Dict[str, Any]) -> int:
        payload = kwargs.get("standard_logging_object")
        if not payload:
            return 0
        events = events_for(payload, self.settings.event_code, self.settings.customer_sources, kwargs)
        if not events:
            log.warning(
                "A call without a customer (looked at %s) is not billed",
                ", ".join(self.settings.customer_sources),
            )
            return 0
        with self._lock:
            self.queue.extend(events)
            self._bound()
        return len(events)

    def _take(self) -> List[Dict[str, Any]]:
        with self._lock:
            count = min(self.settings.batch_size, len(self.queue))
            return [self.queue.popleft() for _ in range(count)]

    def _put_back(self, batch: List[Dict[str, Any]]) -> None:
        with self._lock:
            self.queue.extendleft(reversed(batch))
            self._bound()

    def queued(self) -> int:
        with self._lock:
            return len(self.queue)

    async def async_log_success_event(self, kwargs, response_obj, start_time, end_time):
        self.enqueue(kwargs)
        self._start_flusher()
        if self.queued() >= self.settings.batch_size:
            await self.flush()

    def log_success_event(self, kwargs, response_obj, start_time, end_time):
        self.enqueue(kwargs)
        self._start_sync_flusher()
        if self.queued() >= self.settings.batch_size:
            self.flush_sync()

    async def flush(self) -> None:
        """Sends the queue in batches, each taken off it first; one Meteroid cannot take now goes
        back first in line."""
        while True:
            batch = self._take()
            if not batch:
                return
            try:
                self.rejected += await self._client.ingest(batch)
            except RetryableError as error:
                log.warning("Meteroid did not take %d events (%s): retrying", len(batch), error)
                self._put_back(batch)
                return

    def flush_sync(self) -> None:
        if self._sync_client is None:
            self._sync_client = SyncMeteroidClient(self.settings, self._sync_transport)
        while True:
            batch = self._take()
            if not batch:
                return
            try:
                self.rejected += self._sync_client.ingest(batch)
            except RetryableError as error:
                log.warning("Meteroid did not take %d events (%s): retrying", len(batch), error)
                self._put_back(batch)
                return

    def flush_at_exit(self) -> None:
        self._stopped.set()
        if self.queued():
            try:
                self.flush_sync()
            except Exception:  # The interpreter is going away: never raise from atexit.
                log.exception("Flushing events to Meteroid at exit failed")

    def _start_flusher(self) -> None:
        if self._flusher is None or self._flusher.done():
            self._flusher = asyncio.get_running_loop().create_task(self._periodic_flush())

    def _start_sync_flusher(self) -> None:
        if self._sync_flusher is None or not self._sync_flusher.is_alive():
            self._sync_flusher = threading.Thread(
                target=self._periodic_flush_sync, name="meteroid-flush", daemon=True
            )
            self._sync_flusher.start()

    async def _periodic_flush(self) -> None:
        while True:
            await asyncio.sleep(self.settings.flush_interval)
            try:
                await self.flush()
            except Exception:  # A failed flush must never stop the next ones.
                log.exception("Flushing events to Meteroid failed")

    def _periodic_flush_sync(self) -> None:
        while not self._stopped.wait(self.settings.flush_interval):
            try:
                self.flush_sync()
            except Exception:  # A failed flush must never stop the next ones.
                log.exception("Flushing events to Meteroid failed")

    async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type):
        if self.gate is None:
            return data
        customer = pre_call_customer(user_api_key_dict, data, self.settings.customer_sources)
        if customer and not await self.gate.allows(customer):
            raise refusal(customer)
        return data

    async def aclose(self) -> None:
        self._stopped.set()
        if self._flusher is not None:
            self._flusher.cancel()
        await self.flush()
        await self._client.aclose()
