"""Meteroid's ingest endpoint as far as idempotency goes: an event is kept once per id and
customer, the first copy winning. It refuses the first `FAIL_FIRST` batches with a 503, so the
callback sends the same batch again, and serves what it received on `GET /state`. Its wallet
check answers `has_access` until `POST /exhaust`."""

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FAIL_FIRST = int(os.environ.get("FAIL_FIRST", "2"))
lock = threading.Lock()
deliveries = []
stored = {}
exhausted = threading.Event()


class Meteroid(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path == "/exhaust":
            exhausted.set()
            return self.answer(200, {})
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length))
        with lock:
            deliveries.append({"user_agent": self.headers.get("user-agent"), "body": body})
            if len(deliveries) <= FAIL_FIRST:
                return self.answer(503, {"code": "unavailable"})
            for event in body["events"]:
                stored.setdefault((event["event_id"], event["customer"]), event)
        self.answer(200, {"accepted": len(body["events"]), "rejected": []})

    def do_GET(self):
        if "/wallets/check" in self.path:
            return self.answer(200, {"has_access": not exhausted.is_set(), "exhausted": exhausted.is_set()})
        with lock:
            usage = {}
            for event in stored.values():
                key = f'{event["customer"]}|{event["properties"]["model"]}|{event["properties"]["token_class"]}'
                usage[key] = usage.get(key, 0) + int(event["properties"]["tokens"])
            self.answer(200, {"deliveries": deliveries, "usage": usage, "stored": len(stored)})

    def answer(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("0.0.0.0", 8080), Meteroid).serve_forever()
