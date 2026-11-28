import json
from typing import Any, Dict, List

import httpx
import pytest

from meteroid_litellm.settings import Settings


def payload(call_id: str = "call-1", **overrides: Any) -> Dict[str, Any]:
    """A LiteLLM StandardLoggingPayload, as the proxy logs a successful chat completion."""
    base = {
        "id": "chatcmpl-provider-id",
        "litellm_call_id": call_id,
        "call_type": "acompletion",
        "model": "gpt-4.1-2025-04-14",
        "model_group": "gpt-4.1",
        "custom_llm_provider": "openai",
        "prompt_tokens": 1200,
        "completion_tokens": 350,
        "total_tokens": 1550,
        "response_cost": 0.00001,
        "endTime": 1795000000.25,
        "end_user": "",
        "messages": [{"role": "user", "content": "SECRET PROMPT"}],
        "response": {"choices": [{"message": {"content": "SECRET ANSWER"}}]},
        "metadata": {
            "user_api_key_customer_org_alias": "acme-gmbh",
            "user_api_key_team_alias": "team-a",
            "user_api_key_hash": "sk-hash",
            "usage_object": {
                "prompt_tokens": 1200,
                "completion_tokens": 350,
                "prompt_tokens_details": {"cached_tokens": 200},
                "completion_tokens_details": {"reasoning_tokens": 50},
            },
        },
    }
    base.update(overrides)
    return base


class Recorder:
    """Meteroid's ingest endpoint: records each body, answering with `statuses` in turn."""

    def __init__(self, statuses: List[int] = ()):
        self.statuses = list(statuses)
        self.bodies: List[Dict[str, Any]] = []
        self.requests: List[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        body = json.loads(request.content)
        self.bodies.append(body)
        status = self.statuses.pop(0) if self.statuses else 200
        if status != 200:
            return httpx.Response(status, json={"code": "unavailable"})
        rejected = [
            {"index": i, "path": f"/events/{i}", "event_id": e["event_id"], "message": "Unknown customer"}
            for i, e in enumerate(body["events"])
            if e["customer"] == "ext:unknown"
        ]
        return httpx.Response(200, json={"accepted": len(body["events"]) - len(rejected), "rejected": rejected})


@pytest.fixture
def settings() -> Settings:
    return Settings(api_key="pv_test_key", api_base="http://meteroid.test", batch_size=100)
