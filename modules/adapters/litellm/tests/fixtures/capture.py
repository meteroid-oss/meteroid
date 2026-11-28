"""Captures the StandardLoggingPayload LiteLLM logs for Anthropic, OpenAI and Bedrock calls, each
provider answered by a local mock with its usage fields. Run inside the LiteLLM image:
`docker run --rm -v "$PWD:/c:ro" --entrypoint python ghcr.io/berriai/litellm:main-stable /c/capture.py`."""
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import litellm
from litellm.integrations.custom_logger import CustomLogger

RESPONSES = {
    "anthropic": {"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "SECRET ANSWER"}], "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 100, "output_tokens": 50, "cache_creation_input_tokens": 200, "cache_read_input_tokens": 300}},
    "openai": {"id": "chatcmpl-1", "object": "chat.completion", "created": 1795000000, "model": "o4-mini",
        "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "SECRET ANSWER"}}],
        "usage": {"prompt_tokens": 1000, "completion_tokens": 300, "total_tokens": 1300,
            "prompt_tokens_details": {"cached_tokens": 200}, "completion_tokens_details": {"reasoning_tokens": 100}}},
    "bedrock": {"output": {"message": {"role": "assistant", "content": [{"text": "SECRET ANSWER"}]}}, "stopReason": "end_turn",
        "usage": {"inputTokens": 100, "outputTokens": 50, "totalTokens": 650, "cacheReadInputTokens": 300, "cacheWriteInputTokens": 200},
        "metrics": {"latencyMs": 10}},
}

class Provider(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        kind = "anthropic" if "messages" in self.path and "converse" not in self.path and "chat" not in self.path else ("bedrock" if "converse" in self.path else "openai")
        body = json.dumps(RESPONSES[kind]).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass

server = ThreadingHTTPServer(("127.0.0.1", 8765), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()

captured = {}
class Capture(CustomLogger):
    async def async_log_success_event(self, kwargs, response_obj, start_time, end_time):
        captured[kwargs["litellm_params"]["metadata"]["fixture"]] = kwargs["standard_logging_object"]

litellm.callbacks = [Capture()]
base = "http://127.0.0.1:8765"
async def main():
    msgs = [{"role": "user", "content": "SECRET PROMPT"}]
    await litellm.acompletion(model="anthropic/claude-sonnet-4-5", api_base=base, api_key="x", messages=msgs, metadata={"fixture": "anthropic"})
    await litellm.acompletion(model="openai/o4-mini", api_base=base + "/v1", api_key="x", messages=msgs, metadata={"fixture": "openai"})
    await litellm.acompletion(model="bedrock/converse/anthropic.claude-3-5-sonnet-20240620-v1:0", aws_bedrock_runtime_endpoint=base,
        aws_access_key_id="AKIAX", aws_secret_access_key="x", aws_region_name="eu-west-1", messages=msgs, metadata={"fixture": "bedrock"})
    for _ in range(50):
        if len(captured) == 3: break
        await asyncio.sleep(0.1)
asyncio.run(main())
keep = ("id","litellm_call_id","call_type","model","model_group","custom_llm_provider","prompt_tokens","completion_tokens","total_tokens","response_cost","endTime","end_user")
out = {}
for k, p in captured.items():
    m = p.get("metadata") or {}
    out[k] = {**{f: p.get(f) for f in keep}, "metadata": {"usage_object": m.get("usage_object")}}
print(json.dumps(out, indent=2, default=str))
