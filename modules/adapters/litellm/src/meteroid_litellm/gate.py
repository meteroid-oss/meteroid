"""The optional pre-call gate: refuses a call when the customer's prepaid credit is used up and
usage stops at zero, as Meteroid's wallet check answers, or when Meteroid knows no such customer.
A timeout, a network error, an instance without the wallet check and any other answer follow
`gate_fail_open`."""

from __future__ import annotations

import asyncio
import logging
import time
from typing import Any, Callable, Dict, Mapping, Optional, Tuple

import httpx

from .client import MeteroidClient
from .events import customer_of
from .settings import Settings

log = logging.getLogger("meteroid_litellm")


class CreditExhausted(Exception):
    status_code = 402

    def __init__(self, customer: str):
        super().__init__(f"Meteroid: the prepaid credit of {customer} is used up")
        self.customer = customer


def refusal(customer: str) -> Exception:
    """FastAPI's HTTPException when LiteLLM's proxy runs, which answers the client with a 402."""
    try:
        from fastapi import HTTPException
    except ImportError:
        return CreditExhausted(customer)
    return HTTPException(status_code=402, detail=str(CreditExhausted(customer)))


class CreditGate:
    def __init__(
        self,
        client: MeteroidClient,
        settings: Settings,
        clock: Callable[[], float] = time.monotonic,
    ):
        self.client = client
        self.settings = settings
        self.clock = clock
        self._answers: Dict[str, Tuple[float, bool]] = {}
        self._unknown: set = set()

    async def allows(self, customer: str) -> bool:
        cached = self._answers.get(customer)
        if cached and self.clock() - cached[0] < self.settings.gate_cache_seconds:
            return cached[1]
        try:
            response = await asyncio.wait_for(
                self.client.wallet_check(customer), timeout=self.settings.gate_timeout
            )
        except (asyncio.TimeoutError, httpx.HTTPError) as error:
            log.warning("Meteroid's credit check failed (%s): the call proceeds", error)
            return self.settings.gate_fail_open
        if response.status_code == 404 and _problem(response):
            if customer not in self._unknown:
                self._unknown.add(customer)
                log.warning("Meteroid knows no customer %s: its calls are refused", customer)
            return False
        if response.status_code == 404:
            log.warning("This Meteroid instance has no wallet check: calls follow fail-open")
            return self.settings.gate_fail_open
        if response.status_code != 200:
            log.warning("Meteroid's credit check answered %d", response.status_code)
            return self.settings.gate_fail_open
        allowed = bool(response.json().get("has_access", True))
        self._answers[customer] = (self.clock(), allowed)
        return allowed


def _problem(response: httpx.Response) -> bool:
    return response.headers.get("content-type", "").startswith("application/problem+json")


def _attribute(holder: Any, name: str) -> Any:
    if isinstance(holder, Mapping):
        return holder.get(name)
    return getattr(holder, name, None)


def pre_call_customer(
    user_api_key_dict: Any, data: Mapping[str, Any], sources
) -> Optional[str]:
    """The customer the call's events will name: LiteLLM's logging metadata, as the proxy put it
    on the request before this hook, else built the way LiteLLM builds it from the key."""
    logged = data.get("litellm_metadata") or data.get("metadata") or {}
    customer = _attribute(user_api_key_dict, "customer_hierarchy")
    from_key = {
        "user_api_key_alias": _attribute(user_api_key_dict, "key_alias"),
        "user_api_key_team_id": _attribute(user_api_key_dict, "team_id"),
        "user_api_key_team_alias": _attribute(user_api_key_dict, "team_alias"),
        "user_api_key_user_id": _attribute(user_api_key_dict, "user_id"),
        "user_api_key_end_user_id": _attribute(user_api_key_dict, "end_user_id"),
        "user_api_key_customer_org_alias": _attribute(customer, "customer_org_alias"),
        "user_api_key_customer_member_id": _attribute(customer, "customer_member_id"),
    }
    metadata = {key: logged.get(key) or value for key, value in from_key.items()}
    # As LiteLLM logs it: the key's end user, else the request body's `user`.
    payload = {
        "end_user": metadata["user_api_key_end_user_id"] or data.get("user"),
        "metadata": metadata,
    }
    return customer_of(payload, sources)
