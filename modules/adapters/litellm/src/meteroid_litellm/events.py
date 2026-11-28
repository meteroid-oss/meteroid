"""LiteLLM's logging payload as Meteroid usage events, following the LLM usage convention: one
event per token class of a call, its id `<call id>:<class>`, so a re-sent call is billed once.

Only usage metadata leaves the proxy: never the messages, the response, keys or IP addresses.
"""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from typing import Any, Dict, Iterable, List, Mapping, Optional

TOKEN_CLASSES = ("input", "output", "cache_read", "cache_write", "reasoning")


def decimal_text(value: Any) -> Optional[str]:
    """A number as plain decimal text (`0.00001`, never `1e-05`), which metering parses."""
    if value is None or isinstance(value, bool):
        return None
    try:
        number = Decimal(str(value))
    except (InvalidOperation, ValueError):
        return None
    if not number.is_finite():
        return None
    text = format(number, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in ("", "-0") else text


def _get(mapping: Any, *path: str) -> Any:
    for key in path:
        if not isinstance(mapping, Mapping):
            return None
        mapping = mapping.get(key)
    return mapping


def _int(value: Any) -> int:
    try:
        return max(0, int(value or 0))
    except (TypeError, ValueError):
        return 0


def customer_of(payload: Mapping[str, Any], sources: Iterable[str]) -> Optional[str]:
    """The first identity of `sources` the call carries: what the Meteroid customer's alias is."""
    metadata = payload.get("metadata") or {}
    candidates = {
        "customer_org_alias": metadata.get("user_api_key_customer_org_alias"),
        "customer_member_id": metadata.get("user_api_key_customer_member_id"),
        "end_user": payload.get("end_user") or metadata.get("user_api_key_end_user_id"),
        "team_id": metadata.get("user_api_key_team_id"),
        "team_alias": metadata.get("user_api_key_team_alias"),
        "user_id": metadata.get("user_api_key_user_id"),
        "key_alias": metadata.get("user_api_key_alias"),
    }
    for source in sources:
        value = candidates.get(source)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def token_counts(payload: Mapping[str, Any]) -> Dict[str, int]:
    """Tokens by class. LiteLLM counts cached and cache-written tokens in `prompt_tokens` and
    reasoning tokens in `completion_tokens`: they are split out so no token is counted twice."""
    usage = _get(payload, "metadata", "usage_object") or {}
    prompt = _int(payload.get("prompt_tokens") or usage.get("prompt_tokens"))
    completion = _int(payload.get("completion_tokens") or usage.get("completion_tokens"))
    cache_read = _int(
        _get(usage, "prompt_tokens_details", "cached_tokens") or usage.get("cache_read_input_tokens")
    )
    cache_write = _int(usage.get("cache_creation_input_tokens"))
    reasoning = _int(_get(usage, "completion_tokens_details", "reasoning_tokens"))
    cache_read = min(cache_read, prompt)
    cache_write = min(cache_write, prompt - cache_read)
    reasoning = min(reasoning, completion)
    return {
        "input": prompt - cache_read - cache_write,
        "output": completion - reasoning,
        "cache_read": cache_read,
        "cache_write": cache_write,
        "reasoning": reasoning,
    }


def _timestamp(value: Any) -> Optional[str]:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        moment = datetime.fromtimestamp(float(value), tz=timezone.utc)
    elif isinstance(value, datetime):
        moment = value if value.tzinfo else value.replace(tzinfo=timezone.utc)
    else:
        return None
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def call_id(payload: Mapping[str, Any], kwargs: Optional[Mapping[str, Any]] = None) -> Optional[str]:
    """LiteLLM's id of the call. The payload's `id` can be the provider's response id, which a
    provider may repeat across calls: it is only the last resort."""
    value = (
        payload.get("litellm_call_id")
        or (kwargs or {}).get("litellm_call_id")
        or payload.get("id")
    )
    return str(value) if value else None


def events_for(
    payload: Mapping[str, Any],
    event_code: str,
    customer_sources: Iterable[str],
    kwargs: Optional[Mapping[str, Any]] = None,
) -> List[Dict[str, Any]]:
    """The events of one successful call; none without a call id or a customer."""
    request_id = call_id(payload, kwargs)
    customer = customer_of(payload, customer_sources)
    if not request_id or not customer:
        return []
    model = payload.get("model_group") or payload.get("model") or ""
    provider = payload.get("custom_llm_provider") or ""
    timestamp = _timestamp(payload.get("endTime"))
    cost = decimal_text(payload.get("response_cost"))
    payer = _get(payload, "metadata", "requester_metadata", "payer")

    events: List[Dict[str, Any]] = []
    for token_class, tokens in token_counts(payload).items():
        if tokens <= 0:
            continue
        properties = {
            "model": str(model),
            "provider": str(provider),
            "token_class": token_class,
            "request_id": request_id,
            "tokens": str(tokens),
        }
        # One cost per call, on its first event: the margin spreads it over the call's classes.
        if not events and cost is not None:
            properties["cost_reported"] = cost
            properties["cost_currency"] = "USD"
        if isinstance(payer, str) and payer:
            properties["payer"] = payer
        event = {
            "event_id": f"{request_id}:{token_class}",
            "code": event_code,
            "customer": f"ext:{customer}",
            "properties": properties,
        }
        if timestamp:
            event["timestamp"] = timestamp
        events.append(event)
    return events
