import json

from meteroid_litellm.events import customer_of, decimal_text, events_for, token_counts

from conftest import payload

SOURCES = ("customer_org_alias", "end_user", "team_alias")


def test_one_event_per_token_class_with_ids_derived_from_the_call():
    events = events_for(payload(), "llm_tokens", SOURCES)
    assert [e["event_id"] for e in events] == [
        "call-1:input",
        "call-1:output",
        "call-1:cache_read",
        "call-1:reasoning",
    ]
    assert {e["properties"]["token_class"]: e["properties"]["tokens"] for e in events} == {
        "input": "1000",
        "output": "300",
        "cache_read": "200",
        "reasoning": "50",
    }
    first = events[0]
    assert first["code"] == "llm_tokens"
    assert first["customer"] == "ext:acme-gmbh"
    assert first["timestamp"] == "2026-11-18T11:06:40.250000Z"
    assert first["properties"]["model"] == "gpt-4.1"
    assert first["properties"]["provider"] == "openai"
    assert first["properties"]["request_id"] == "call-1"


def test_the_call_id_is_litellm_s_not_the_provider_response_id():
    first = events_for(payload("call-1"), "llm_tokens", SOURCES)
    second = events_for(payload("call-2"), "llm_tokens", SOURCES)
    assert first[0]["event_id"] == "call-1:input"
    assert second[0]["event_id"] == "call-2:input", "both share the provider's response id"
    legacy = payload("call-1", litellm_call_id=None)
    assert events_for(legacy, "llm_tokens", SOURCES, {"litellm_call_id": "kw-1"})[0]["event_id"] == "kw-1:input"


def test_the_same_call_always_yields_the_same_events():
    assert events_for(payload(), "llm_tokens", SOURCES) == events_for(payload(), "llm_tokens", SOURCES)


def test_the_reported_cost_is_sent_once_per_call_as_plain_decimal_text():
    events = events_for(payload(response_cost=0.00001), "llm_tokens", SOURCES)
    costs = [e["properties"].get("cost_reported") for e in events]
    assert costs == ["0.00001", None, None, None]
    assert events[0]["properties"]["cost_currency"] == "USD"
    assert decimal_text(1e-7) == "0.0000001"
    assert decimal_text(2.50) == "2.5"
    assert decimal_text(3) == "3"
    assert decimal_text(None) is None
    assert decimal_text(float("nan")) is None


def test_no_prompt_or_completion_leaves_the_proxy():
    sent = json.dumps(events_for(payload(), "llm_tokens", SOURCES))
    assert "SECRET" not in sent
    assert "sk-hash" not in sent


def test_the_customer_is_the_first_identity_the_call_carries():
    p = payload()
    assert customer_of(p, SOURCES) == "acme-gmbh"
    p["metadata"]["user_api_key_customer_org_alias"] = None
    p["end_user"] = "end-user-7"
    assert customer_of(p, SOURCES) == "end-user-7"
    p["end_user"] = ""
    assert customer_of(p, SOURCES) == "team-a"
    assert customer_of(p, ("key_alias",)) is None


def test_a_call_without_a_customer_or_an_id_is_not_billed():
    p = payload()
    p["metadata"] = {"usage_object": {}}
    assert events_for(p, "llm_tokens", SOURCES) == []
    assert events_for(payload(id=None, litellm_call_id=None), "llm_tokens", SOURCES) == []


def test_cache_writes_and_reads_are_not_counted_as_input_twice():
    p = payload(prompt_tokens=1000, completion_tokens=10)
    p["metadata"]["usage_object"] = {
        "cache_read_input_tokens": 300,
        "cache_creation_input_tokens": 500,
    }
    assert token_counts(p) == {
        "input": 200,
        "output": 10,
        "cache_read": 300,
        "cache_write": 500,
        "reasoning": 0,
    }
