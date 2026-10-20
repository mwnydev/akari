#!/usr/bin/env python3
"""本机 JSON-RPC 计数转发：把请求原样转给上游 RPC，按方法统计次数，用来估算 RPC 服务商额度（Infura credits 等）。

  RPC_METER_UPSTREAM=<上游 URL> deploy/testnet/rpc-meter.py <端口> <计数文件.json>

上游 URL 带服务商密钥，只从环境变量读（不进命令行参数、不写日志、错误信息里也不出现）。只听 127.0.0.1；
出网走环境里的 HTTPS_PROXY。计数文件每次请求后改写：{"methods": {方法: 次数}, "requests": HTTP 请求数}。
"""

import json
import os
import signal
import sys
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = os.environ.pop("RPC_METER_UPSTREAM", "")
LOCK = threading.Lock()
COUNTS: dict[str, int] = {}
REQUESTS = 0


def save(path: str) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"methods": dict(sorted(COUNTS.items())), "requests": REQUESTS}, f, indent=2)
    os.replace(tmp, path)


def methods(body: bytes) -> list[str]:
    try:
        msg = json.loads(body)
    except ValueError:
        return ["<invalid>"]
    items = msg if isinstance(msg, list) else [msg]
    return [str(m.get("method", "<none>")) if isinstance(m, dict) else "<invalid>" for m in items]


class Handler(BaseHTTPRequestHandler):
    counts_path = ""

    def do_POST(self) -> None:  # noqa: N802 (http.server 的命名)
        global REQUESTS
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        with LOCK:
            REQUESTS += 1
            for m in methods(body):
                COUNTS[m] = COUNTS.get(m, 0) + 1
            save(self.counts_path)
        # 公共 RPC（Cloudflare）拒绝 urllib 默认的 User-Agent（error 1010）
        headers = {"content-type": "application/json", "user-agent": "akari-rpc-meter/1"}
        req = urllib.request.Request(UPSTREAM, data=body, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                status, data = resp.status, resp.read()
        except urllib.error.HTTPError as e:  # 上游的 4xx / 5xx 原样转回
            status, data = e.code, e.read()
        except Exception as e:  # 连接失败：只报异常类型，不带 URL
            status = 502
            data = json.dumps({"jsonrpc": "2.0", "id": None, "error": {"code": -32603, "message": f"upstream {type(e).__name__}"}}).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_args) -> None:  # 不打访问日志
        pass


def main() -> None:
    if len(sys.argv) != 3 or not UPSTREAM.startswith(("https://", "http://")):
        sys.exit("usage: RPC_METER_UPSTREAM=<url> rpc-meter.py <port> <counts.json>")
    Handler.counts_path = sys.argv[2]
    save(Handler.counts_path)
    server = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=server.shutdown).start())
    server.serve_forever()


if __name__ == "__main__":
    main()
