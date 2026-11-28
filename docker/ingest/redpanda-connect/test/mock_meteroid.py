"""v2 ingest as the recipes see it: 429 once, `ext:unknown` rejected, accepted events appended to a file."""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

ACCEPTED = os.environ.get("ACCEPTED_PATH", "/data/accepted.jsonl")
FIELDS = {"event_id", "code", "customer", "timestamp", "properties"}
state = {"requests": 0}


class Handler(BaseHTTPRequestHandler):
    def answer(self, status, body, headers=()):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        if self.path != "/api/v2/events/ingest":
            return self.answer(404, {"code": "not_found"})
        if self.headers.get("Authorization") != "Bearer test-key":
            return self.answer(401, {"code": "unauthorized"})
        state["requests"] += 1
        if state["requests"] == 1:
            return self.answer(429, {"code": "rate_limited"}, [("Retry-After", "1")])
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        events = body["events"]
        if not body.get("allow_partial_failures") or not 1 <= len(events) <= 500:
            return self.answer(422, {"code": "validation_failed"})
        if any(set(event) - FIELDS for event in events):
            return self.answer(422, {"code": "validation_failed", "detail": "unknown field"})
        rejected = [
            {"index": i, "path": f"/events/{i}", "event_id": e["event_id"], "message": "Unknown customer alias"}
            for i, e in enumerate(events)
            if e["customer"] == "ext:unknown"
        ]
        refused = {r["index"] for r in rejected}
        with open(ACCEPTED, "a") as out:
            for i, event in enumerate(events):
                if i not in refused:
                    out.write(json.dumps(event) + "\n")
        self.answer(200, {"accepted": len(events) - len(rejected), "rejected": rejected})

    def log_message(self, *args):
        sys.stderr.write("%s\n" % (args[0] % args[1:]))


HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
