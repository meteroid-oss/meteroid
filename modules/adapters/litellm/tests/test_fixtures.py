"""Token splits of payloads LiteLLM 1.111.0 logged for real provider usage fields
(`fixtures/litellm-1.111.0-usage.json`, made by `fixtures/capture.py`)."""

import json
from pathlib import Path

import pytest

from meteroid_litellm.events import events_for, token_counts

FIXTURES = json.loads((Path(__file__).parent / "fixtures" / "litellm-1.111.0-usage.json").read_text())


def test_the_fixtures_are_pinned_to_a_litellm_version():
    assert FIXTURES["litellm_version"] == "1.111.0"


@pytest.mark.parametrize(
    "provider, counts",
    [
        # input_tokens 100, cache_creation 200, cache_read 300: LiteLLM's prompt_tokens is 600.
        ("anthropic", {"input": 100, "output": 50, "cache_read": 300, "cache_write": 200, "reasoning": 0}),
        ("bedrock", {"input": 100, "output": 50, "cache_read": 300, "cache_write": 200, "reasoning": 0}),
        # prompt 1000 of which 200 cached, completion 300 of which 100 reasoning.
        ("openai", {"input": 800, "output": 200, "cache_read": 200, "cache_write": 0, "reasoning": 100}),
    ],
)
def test_every_token_is_counted_once_in_its_class(provider, counts):
    payload = FIXTURES["payloads"][provider]
    assert token_counts(payload) == counts
    assert sum(counts.values()) == payload["prompt_tokens"] + payload["completion_tokens"]


def test_a_fixture_becomes_one_event_per_class_with_tokens():
    payload = dict(FIXTURES["payloads"]["anthropic"], end_user="acme-gmbh")
    events = events_for(payload, "llm_tokens", ("end_user",))
    assert [e["properties"]["token_class"] for e in events] == ["input", "output", "cache_read", "cache_write"]
    assert events[0]["event_id"] == f'{payload["litellm_call_id"]}:input'
    assert events[0]["properties"]["model"] == "claude-sonnet-4-5", "an empty model group falls back to the model"
    assert events[0]["properties"]["cost_reported"] == "0.00189"
