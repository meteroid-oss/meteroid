import asyncio
from dataclasses import replace
from types import SimpleNamespace

import httpx
import pytest

from meteroid_litellm.gate import CreditExhausted
from meteroid_litellm.logger import MeteroidLogger


class Wallet:
    """Meteroid's wallet check: `has_access` until `exhausted` is set."""

    def __init__(self, status=200, delay=0.0, problem=True):
        self.status = status
        self.delay = delay
        self.problem = problem
        self.exhausted = False
        self.calls = []

    async def __call__(self, request):
        self.calls.append(request)
        if self.delay:
            await asyncio.sleep(self.delay)
        if self.status != 200 and not self.problem:
            return httpx.Response(self.status, text="Not Found")
        if self.status != 200:
            headers = {"content-type": "application/problem+json"}
            return httpx.Response(self.status, json={"code": "not_found"}, headers=headers)
        return httpx.Response(200, json={"has_access": not self.exhausted, "exhausted": self.exhausted})


def gated(settings, wallet, **overrides):
    return MeteroidLogger(
        replace(settings, credit_gate=True, gate_currency="EUR", **overrides),
        transport=httpx.MockTransport(wallet),
    )


def caller(alias="acme-gmbh"):
    return SimpleNamespace(team_alias=None, team_id=None, user_id=None, key_alias=None, end_user_id=None), {
        "model": "gpt-4.1",
        "user": alias,
        "messages": [],
    }


async def test_a_customer_with_credit_is_let_through(settings):
    wallet = Wallet()
    logger = gated(settings, wallet)
    key, data = caller()
    assert await logger.async_pre_call_hook(key, None, data, "completion") is data
    request = wallet.calls[0]
    assert request.url.path == "/api/v2/customers/ext:acme-gmbh/wallets/check"
    assert request.url.params["currency"] == "EUR"


async def test_an_exhausted_wallet_is_refused_once_the_cached_answer_expires(settings):
    wallet = Wallet()
    logger = gated(settings, wallet, gate_cache_seconds=0.05)
    key, data = caller()
    await logger.async_pre_call_hook(key, None, data, "completion")
    wallet.exhausted = True
    await logger.async_pre_call_hook(key, None, data, "completion")
    assert len(wallet.calls) == 1, "answered from the cache"
    await asyncio.sleep(0.06)
    with pytest.raises(Exception) as refused:
        await logger.async_pre_call_hook(key, None, data, "completion")
    assert getattr(refused.value, "status_code", None) == 402 or isinstance(refused.value, CreditExhausted)


async def test_an_unreachable_meteroid_lets_the_call_through_within_the_timeout(settings):
    logger = gated(settings, Wallet(delay=5), gate_timeout=0.1)
    key, data = caller()
    started = asyncio.get_running_loop().time()
    assert await logger.async_pre_call_hook(key, None, data, "completion") is data
    assert asyncio.get_running_loop().time() - started < 1


async def test_fail_closed_when_asked(settings):
    logger = gated(settings, Wallet(delay=5), gate_timeout=0.05, gate_fail_open=False)
    key, data = caller()
    with pytest.raises(Exception):
        await logger.async_pre_call_hook(key, None, data, "completion")


async def test_an_unknown_customer_is_refused_even_failing_open(settings):
    key, data = caller()
    with pytest.raises(Exception):
        await gated(settings, Wallet(status=404)).async_pre_call_hook(key, None, data, "completion")


async def test_an_instance_without_the_wallet_check_follows_fail_open(settings):
    key, data = caller()
    logger = gated(settings, Wallet(status=404, problem=False))
    assert await logger.async_pre_call_hook(key, None, data, "completion") is data
    logger = gated(settings, Wallet(status=404, problem=False), gate_fail_open=False)
    with pytest.raises(Exception):
        await logger.async_pre_call_hook(key, None, data, "completion")


async def test_any_other_answer_follows_fail_open(settings):
    key, data = caller()
    assert await gated(settings, Wallet(status=500)).async_pre_call_hook(key, None, data, "c") is data
    with pytest.raises(Exception):
        await gated(settings, Wallet(status=500), gate_fail_open=False).async_pre_call_hook(
            key, None, data, "c"
        )


async def test_without_the_gate_nothing_is_checked(settings):
    wallet = Wallet()
    logger = MeteroidLogger(settings, transport=httpx.MockTransport(wallet))
    key, data = caller()
    assert await logger.async_pre_call_hook(key, None, data, "completion") is data
    assert wallet.calls == []


SOURCES = ("customer_org_alias", "end_user", "team_alias")


@pytest.mark.parametrize(
    "logged, key, body_user, expected",
    [
        ({"user_api_key_customer_org_alias": "acme-org"}, {}, "bob", "acme-org"),
        ({}, {"end_user_id": "eu-1"}, "bob", "eu-1"),
        ({}, {}, "bob", "bob"),
        ({"user_api_key_team_alias": "team-a"}, {}, None, "team-a"),
        ({}, {"customer_hierarchy": SimpleNamespace(customer_org_alias="from-key", customer_member_id=None)}, None, "from-key"),
        ({}, {}, None, None),
    ],
)
def test_the_gate_names_the_customer_the_events_will(logged, key, body_user, expected):
    from meteroid_litellm.events import customer_of
    from meteroid_litellm.gate import pre_call_customer

    key_auth = SimpleNamespace(
        **{name: None for name in ("key_alias", "team_id", "team_alias", "user_id", "end_user_id", "customer_hierarchy")}
        | key
    )
    data = {"user": body_user, "metadata": dict(logged)} if body_user else {"metadata": dict(logged)}
    # What LiteLLM logs for the same call: its metadata, and the key's end user else the body's user.
    hierarchy = key.get("customer_hierarchy")
    metadata = {
        "user_api_key_end_user_id": key.get("end_user_id"),
        "user_api_key_customer_org_alias": getattr(hierarchy, "customer_org_alias", None),
        **logged,
    }
    logging_payload = {"end_user": metadata["user_api_key_end_user_id"] or body_user, "metadata": metadata}
    assert pre_call_customer(key_auth, data, SOURCES) == customer_of(logging_payload, SOURCES) == expected
