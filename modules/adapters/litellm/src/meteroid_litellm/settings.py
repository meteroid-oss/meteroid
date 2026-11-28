"""Settings read from the proxy's environment."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Mapping, Optional, Tuple

DEFAULT_API_BASE = "https://api.meteroid.com"
DEFAULT_EVENT_CODE = "llm_tokens"
DEFAULT_CUSTOMER_SOURCES = ("customer_org_alias", "end_user", "team_alias")

CUSTOMER_SOURCES = (
    "customer_org_alias",
    "customer_member_id",
    "end_user",
    "team_id",
    "team_alias",
    "user_id",
    "key_alias",
)


def _flag(value: Optional[str], default: bool) -> bool:
    if value is None or value.strip() == "":
        return default
    return value.strip().lower() in ("1", "true", "yes", "on")


def _number(value: Optional[str], default: float) -> float:
    try:
        return float(value) if value not in (None, "") else default
    except ValueError:
        return default


@dataclass(frozen=True)
class Settings:
    api_key: str
    api_base: str = DEFAULT_API_BASE
    event_code: str = DEFAULT_EVENT_CODE
    customer_sources: Tuple[str, ...] = DEFAULT_CUSTOMER_SOURCES
    batch_size: int = 100
    flush_interval: float = 5.0
    request_timeout: float = 10.0
    max_queue: int = 50_000
    credit_gate: bool = False
    gate_currency: Optional[str] = None
    gate_credit_unit: Optional[str] = None
    gate_timeout: float = 1.0
    gate_fail_open: bool = True
    gate_cache_seconds: float = 1.0
    extra_headers: Mapping[str, str] = field(default_factory=dict)

    @staticmethod
    def from_env(env: Optional[Mapping[str, str]] = None) -> "Settings":
        env = os.environ if env is None else env
        sources = tuple(
            source.strip()
            for source in env.get("METEROID_CUSTOMER_ID_FROM", ",".join(DEFAULT_CUSTOMER_SOURCES)).split(",")
            if source.strip() in CUSTOMER_SOURCES
        )
        return Settings(
            api_key=env.get("METEROID_API_KEY", ""),
            api_base=env.get("METEROID_API_BASE", DEFAULT_API_BASE).rstrip("/"),
            event_code=env.get("METEROID_EVENT_CODE", DEFAULT_EVENT_CODE),
            customer_sources=sources or DEFAULT_CUSTOMER_SOURCES,
            batch_size=max(1, min(500, int(_number(env.get("METEROID_BATCH_SIZE"), 100)))),
            flush_interval=max(0.1, _number(env.get("METEROID_FLUSH_INTERVAL"), 5.0)),
            request_timeout=_number(env.get("METEROID_TIMEOUT"), 10.0),
            credit_gate=_flag(env.get("METEROID_CREDIT_GATE"), False),
            gate_currency=env.get("METEROID_CREDIT_GATE_CURRENCY") or None,
            gate_credit_unit=env.get("METEROID_CREDIT_GATE_CREDIT_UNIT") or None,
            gate_timeout=_number(env.get("METEROID_CREDIT_GATE_TIMEOUT"), 1.0),
            gate_fail_open=_flag(env.get("METEROID_CREDIT_GATE_FAIL_OPEN"), True),
        )
