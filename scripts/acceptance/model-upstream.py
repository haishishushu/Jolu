"""统一网关真实桌面验收用的合成模型上游：只在本机回环监听，不调用外网、不读取用户密钥。

支持 Anthropic Messages、OpenAI Chat Completions / Responses 与 Gemini 原生 generateContent（含流式），
回复固定文本并给出合成用量。每个请求的路径、请求头（凭据类请求头脱敏）与请求体顶层字段写入
output/acceptance/upstream-requests.jsonl，用来核对各工具实际携带的会话标识。

用法：python -I scripts/acceptance/model-upstream.py [端口，默认 18766]
环境变量 JOLU_UPSTREAM_DELAY：回复前等待的秒数（默认 4），便于在界面上观察「运行中」。
"""
import json
import os
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18766
DELAY = float(os.environ.get("JOLU_UPSTREAM_DELAY", "4"))
LOG = os.path.join(os.path.dirname(__file__), "..", "..", "output", "acceptance", "upstream-requests.jsonl")
SECRET_HEADERS = {"authorization", "x-api-key", "x-goog-api-key", "cookie", "proxy-authorization"}
TEXT = "OK"


def record(handler, body):
    headers = {k.lower(): ("<redacted>" if k.lower() in SECRET_HEADERS else v) for k, v in handler.headers.items()}
    keys = sorted(body.keys()) if isinstance(body, dict) else []
    meta = {k: body.get(k) for k in ("model", "metadata", "prompt_cache_key", "session_id", "user", "stream") if isinstance(body, dict) and k in body}
    os.makedirs(os.path.dirname(LOG), exist_ok=True)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps({"ts": time.time(), "method": handler.command, "path": handler.path, "headers": headers, "body_keys": keys, "body_meta": meta}, ensure_ascii=False) + "\n")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def _json(self, status, data):
        raw = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _sse(self, events, done=False):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for name, data in events:
            chunk = (f"event: {name}\n" if name else "") + f"data: {json.dumps(data)}\n\n"
            self.wfile.write(chunk.encode())
            self.wfile.flush()
        if done:
            self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True

    def do_GET(self):
        record(self, None)
        if "models" in self.path:
            if self.path.startswith("/v1beta"):
                return self._json(200, {"models": [{"name": "models/test-model", "displayName": "test-model", "supportedGenerationMethods": ["generateContent"]}]})
            return self._json(200, {"object": "list", "data": [{"id": "test-model", "object": "model", "owned_by": "acceptance"}]})
        return self._json(404, {"error": "not found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            body = {}
        record(self, body)
        path = self.path.split("?")[0]
        if path.endswith(":countTokens"):
            return self._json(200, {"totalTokens": 10})
        time.sleep(DELAY)
        stream = bool(body.get("stream")) or "streamGenerateContent" in path
        model = body.get("model", "test-model")
        if path.endswith("/messages"):
            return self.reply_anthropic(model, stream)
        if path.endswith("/chat/completions"):
            return self.reply_chat(model, stream)
        if path.endswith("/responses"):
            return self.reply_responses(model, stream)
        if ":generateContent" in path or ":streamGenerateContent" in path:
            return self.reply_gemini(stream)
        return self._json(404, {"error": f"unsupported path {path}"})

    def reply_anthropic(self, model, stream):
        mid = f"msg_{uuid.uuid4().hex[:12]}"
        usage = {"input_tokens": 12, "output_tokens": 3, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}
        if not stream:
            return self._json(200, {"id": mid, "type": "message", "role": "assistant", "model": model, "content": [{"type": "text", "text": TEXT}], "stop_reason": "end_turn", "stop_sequence": None, "usage": usage})
        self._sse([
            ("message_start", {"type": "message_start", "message": {"id": mid, "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": {**usage, "output_tokens": 1}}}),
            ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": TEXT}}),
            ("content_block_stop", {"type": "content_block_stop", "index": 0}),
            ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 3}}),
            ("message_stop", {"type": "message_stop"}),
        ])

    def reply_chat(self, model, stream):
        cid = f"chatcmpl-{uuid.uuid4().hex[:12]}"
        usage = {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
        if not stream:
            return self._json(200, {"id": cid, "object": "chat.completion", "created": int(time.time()), "model": model, "choices": [{"index": 0, "message": {"role": "assistant", "content": TEXT}, "finish_reason": "stop"}], "usage": usage})
        base = {"id": cid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model}
        self._sse([
            (None, {**base, "choices": [{"index": 0, "delta": {"role": "assistant", "content": TEXT}, "finish_reason": None}]}),
            (None, {**base, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
            (None, {**base, "choices": [], "usage": usage}),
        ], done=True)

    def reply_responses(self, model, stream):
        rid = f"resp_{uuid.uuid4().hex[:12]}"
        item = {"id": f"msg_{uuid.uuid4().hex[:12]}", "type": "message", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": TEXT, "annotations": []}]}
        usage = {"input_tokens": 12, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 3, "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 15}
        done = {"id": rid, "object": "response", "created_at": int(time.time()), "status": "completed", "model": model, "output": [item], "usage": usage}
        if not stream:
            return self._json(200, done)
        self._sse([
            ("response.created", {"type": "response.created", "response": {**done, "status": "in_progress", "output": [], "usage": None}}),
            ("response.output_item.added", {"type": "response.output_item.added", "output_index": 0, "item": {**item, "status": "in_progress", "content": []}}),
            ("response.output_text.delta", {"type": "response.output_text.delta", "item_id": item["id"], "output_index": 0, "content_index": 0, "delta": TEXT}),
            ("response.output_text.done", {"type": "response.output_text.done", "item_id": item["id"], "output_index": 0, "content_index": 0, "text": TEXT}),
            ("response.output_item.done", {"type": "response.output_item.done", "output_index": 0, "item": item}),
            ("response.completed", {"type": "response.completed", "response": done}),
        ])

    def reply_gemini(self, stream):
        data = {"candidates": [{"content": {"role": "model", "parts": [{"text": TEXT}]}, "finishReason": "STOP", "index": 0}],
                "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 3, "totalTokenCount": 15}, "modelVersion": "test-model"}
        if not stream:
            return self._json(200, data)
        self._sse([(None, data)])


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
