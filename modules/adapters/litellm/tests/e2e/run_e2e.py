"""Sends calls through the proxy, then checks what the mock Meteroid received: the batch it
refused twice came back three times with the same event ids, and the usage it kept is that of
one delivery. Then the gate: an exhausted wallet is refused within 5 s, and with Meteroid down a
call proceeds within the gate's timeout. Run after `docker compose up -d` in this directory."""

import json
import subprocess
import sys
import urllib.error
import time
import urllib.request

PROXY = "http://localhost:14000"
METEROID = "http://localhost:18080"
CALLS = 5


def request(url, body=None):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"content-type": "application/json", "authorization": "Bearer sk-e2e"}
    req = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.loads(response.read())


def wait_for_proxy():
    for _ in range(120):
        try:
            urllib.request.urlopen(f"{PROXY}/health/liveliness", timeout=2)
            return
        except Exception:
            time.sleep(1)
    sys.exit("the proxy did not start")


def call(i):
    return request(
        f"{PROXY}/v1/chat/completions",
        {
            "model": "gpt-mock",
            "user": "acme-gmbh",
            "messages": [{"role": "user", "content": f"SECRET PROMPT {i}"}],
        },
    )


def check_gate():
    request(f"{METEROID}/exhaust", {})
    exhausted_at = time.monotonic()
    while True:
        try:
            call("after exhaustion")
        except urllib.error.HTTPError as error:
            assert error.code == 402, error.code
            break
        assert time.monotonic() - exhausted_at < 5, "an exhausted wallet is refused within 5 s"
        time.sleep(0.2)
    refused_after = time.monotonic() - exhausted_at
    subprocess.run(["docker", "compose", "stop", "meteroid"], check=True, capture_output=True)
    started = time.monotonic()
    call("meteroid down")
    proceeded_after = time.monotonic() - started
    assert proceeded_after < 5, proceeded_after
    print(f"ok: refused {refused_after:.1f}s after exhaustion; with Meteroid down a call took {proceeded_after:.1f}s")


def main():
    wait_for_proxy()
    for i in range(CALLS):
        call(i)
    state = None
    for _ in range(60):
        state = request(f"{METEROID}/state")
        if state["stored"] >= CALLS * 2:
            break
        time.sleep(1)
    deliveries = state["deliveries"]
    ids = [{e["event_id"] for e in d["body"]["events"]} for d in deliveries]
    assert len(deliveries) >= 3, f"expected the refused batch to come back, got {len(deliveries)}"
    # A batch sent again carries the same ids, and the calls logged meanwhile after them.
    assert ids[0] <= ids[1] <= ids[2], "a re-sent batch keeps its event ids"
    assert state["usage"] == {
        "ext:acme-gmbh|gpt-mock|input": 120 * CALLS,
        "ext:acme-gmbh|gpt-mock|output": 30 * CALLS,
    }, state["usage"]
    assert "SECRET" not in json.dumps(deliveries), "no prompt or completion leaves the proxy"
    assert deliveries[0]["user_agent"].startswith("meteroid-litellm/")
    print(f"ok: {len(deliveries)} deliveries, {state['stored']} events kept, usage {state['usage']}")
    check_gate()


if __name__ == "__main__":
    main()
