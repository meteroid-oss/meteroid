"""Meteroid's REST API, as the callback calls it."""

from __future__ import annotations

import logging
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, List, Optional
from urllib.parse import quote

import httpx

from . import __version__
from .settings import Settings

log = logging.getLogger("meteroid_litellm")

# Metering accepts events up to 24 hours old without backfilling.
BACKFILL_AFTER = timedelta(hours=23)


class RetryableError(Exception):
    """Meteroid could not take the batch now: it is sent again, with the same event ids."""


def _headers(settings: Settings) -> Dict[str, str]:
    return {
        "Authorization": f"Bearer {settings.api_key}",
        "User-Agent": f"meteroid-litellm/{__version__}",
        **settings.extra_headers,
    }


def _needs_backfill(events: List[Dict[str, Any]], now: datetime) -> bool:
    for event in events:
        stamp = event.get("timestamp")
        if not stamp:
            continue
        try:
            at = datetime.strptime(stamp, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc)
        except ValueError:
            continue
        if now - at > BACKFILL_AFTER:
            return True
    return False


def ingest_body(events: List[Dict[str, Any]], now: Optional[datetime] = None) -> Dict[str, Any]:
    return {
        "events": events,
        "allow_partial_failures": True,
        "allow_backfilling": _needs_backfill(events, now or datetime.now(timezone.utc)),
    }


def _handle_ingest(response: httpx.Response, sent: int) -> int:
    """How many events Meteroid rejected for good; raises when the batch is to be sent again."""
    # A key refused (401, 403) is fixed by the operator; its events wait for it.
    if response.status_code in (401, 403, 429) or response.status_code >= 500:
        raise RetryableError(f"Meteroid answered {response.status_code}")
    if response.status_code >= 400:
        log.error(
            "Meteroid refused a batch of %d events (%d), dropped: %s",
            sent,
            response.status_code,
            response.text[:500],
        )
        return sent
    rejected = (response.json() or {}).get("rejected") or []
    for rejection in rejected:
        log.warning(
            "Meteroid rejected event %s: %s",
            rejection.get("event_id"),
            rejection.get("message"),
        )
    return len(rejected)


class MeteroidClient:
    def __init__(self, settings: Settings, transport: Optional[httpx.AsyncBaseTransport] = None):
        self.settings = settings
        self._client = httpx.AsyncClient(
            base_url=settings.api_base,
            headers=_headers(settings),
            timeout=settings.request_timeout,
            transport=transport,
        )

    async def ingest(self, events: List[Dict[str, Any]]) -> int:
        try:
            response = await self._client.post("/api/v2/events/ingest", json=ingest_body(events))
        except httpx.HTTPError as error:
            raise RetryableError(str(error)) from error
        return _handle_ingest(response, len(events))

    async def wallet_check(self, customer: str) -> httpx.Response:
        params = {}
        if self.settings.gate_credit_unit:
            params["credit_unit"] = self.settings.gate_credit_unit
        elif self.settings.gate_currency:
            params["currency"] = self.settings.gate_currency
        return await self._client.get(
            f"/api/v2/customers/{quote('ext:' + customer, safe=':')}/wallets/check",
            params=params,
            timeout=self.settings.gate_timeout,
        )

    async def aclose(self) -> None:
        await self._client.aclose()


class SyncMeteroidClient:
    """For LiteLLM's synchronous SDK calls, which log outside an event loop."""

    def __init__(self, settings: Settings, transport: Optional[httpx.BaseTransport] = None):
        self._client = httpx.Client(
            base_url=settings.api_base,
            headers=_headers(settings),
            timeout=settings.request_timeout,
            transport=transport,
        )

    def ingest(self, events: List[Dict[str, Any]]) -> int:
        try:
            response = self._client.post("/api/v2/events/ingest", json=ingest_body(events))
        except httpx.HTTPError as error:
            raise RetryableError(str(error)) from error
        return _handle_ingest(response, len(events))
