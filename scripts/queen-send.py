#!/usr/bin/env python3
"""queen-send: Queen MCP サーバー経由でエージェントペインにメッセージを送り、返答を待って表示する。

使い方:
  queen-send.py <agent> <message>            # 送信 → 返答が安定するまで待機 → 出力表示
  queen-send.py <agent> --read               # 読むだけ(送信しない)
  queen-send.py <agent> <message> --no-wait  # 送信だけして即終了
  queen-send.py <agent> --enter              # Enterのみ送出(未送信テキストの送信/ダイアログ突破)

オプション:
  --lines N      read_output の行数 (default 300)
  --timeout SEC  返答待ちの最大秒数 (default 600)
  --interval SEC ポーリング間隔秒 (default 10)
  --nudge SEC    送信後この秒数出力に変化がなければ Enter を追加送出 (default 6, 0で無効)

仕組み:
  - ポート 39237..39246 を順に試して Queen を発見
  - 送信後、出力が送信前スナップショットから変化しないままなら Enter を1回だけ追加送出
    (TUIのアップデート確認ダイアログ等が最初の Enter を消費するケース対策)
  - 出力が2回連続で変化しなくなったら完了とみなし、全文を stdout に出す
"""
import argparse
import json
import sys
import time
import urllib.request
import urllib.error

PORTS = range(39237, 39247)


def sse_json(body: bytes):
    """SSE または素のJSONレスポンスから最初の JSON-RPC data を取り出す。"""
    text = body.decode("utf-8", "replace")
    for line in text.splitlines():
        if line.startswith("data: {"):
            return json.loads(line[len("data: "):])
    return json.loads(text)


class Queen:
    def __init__(self):
        self.url = None
        self.sid = None
        self._connect()

    def _post(self, payload, extra_headers=None):
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.sid:
            headers["Mcp-Session-Id"] = self.sid
        if extra_headers:
            headers.update(extra_headers)
        req = urllib.request.Request(self.url, data=json.dumps(payload).encode(), headers=headers)
        return urllib.request.urlopen(req, timeout=15)

    def _connect(self):
        init = {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                       "clientInfo": {"name": "queen-send", "version": "1.0"}},
        }
        for port in PORTS:
            self.url = f"http://127.0.0.1:{port}/mcp"
            try:
                resp = self._post(init)
                self.sid = resp.headers.get("Mcp-Session-Id")
                sse_json(resp.read())
                self._post({"jsonrpc": "2.0", "method": "notifications/initialized"}).read()
                return
            except (urllib.error.URLError, OSError):
                continue
        sys.exit("error: Queen サーバーが見つかりません (ports 39237-39246)。アプリが起動しているか確認してください。")

    def call(self, tool, args):
        resp = self._post({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                           "params": {"name": tool, "arguments": args}})
        data = sse_json(resp.read())
        if "error" in data:
            sys.exit(f"error: {data['error']}")
        result = data["result"]
        content = result.get("content", [{}])[0].get("text", "")
        if result.get("isError"):
            sys.exit(f"error from queen: {content}")
        return content

    def read_output(self, agent, lines):
        raw = self.call("read_output", {"agent": agent, "lines": lines})
        try:
            return json.loads(raw)["text"]
        except (json.JSONDecodeError, KeyError):
            return raw

    def send(self, agent, text, submit=True):
        return self.call("send_message", {"agent": agent, "text": text, "submit": submit})


def main():
    ap = argparse.ArgumentParser(description="Send a message to an agent pane via Queen and wait for the reply.")
    ap.add_argument("agent")
    ap.add_argument("message", nargs="?")
    ap.add_argument("--read", action="store_true", help="read_output のみ実行")
    ap.add_argument("--enter", action="store_true", help="Enter のみ送出")
    ap.add_argument("--no-wait", action="store_true", help="送信のみで待機しない")
    ap.add_argument("--lines", type=int, default=300)
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--interval", type=int, default=10)
    ap.add_argument("--nudge", type=int, default=6)
    args = ap.parse_args()

    q = Queen()

    if args.read:
        print(q.read_output(args.agent, args.lines))
        return
    if args.enter:
        q.send(args.agent, "", submit=True)
        print("enter sent", file=sys.stderr)
        return
    if not args.message:
        ap.error("message が必要です (--read / --enter を除く)")

    before = q.read_output(args.agent, args.lines)
    q.send(args.agent, args.message, submit=True)
    print(f"sent to {args.agent}", file=sys.stderr)
    if args.no_wait:
        return

    # ナッジ: 送信後しばらく出力が変化しなければ、composer に残った未送信
    # テキストやダイアログを Enter で送り出す(1回だけ)
    if args.nudge > 0:
        time.sleep(args.nudge)
        if q.read_output(args.agent, args.lines) == before:
            q.send(args.agent, "", submit=True)
            print("no activity — sent extra Enter", file=sys.stderr)

    deadline = time.time() + args.timeout
    prev, changed, stable = before, False, 0
    while time.time() < deadline:
        time.sleep(args.interval)
        cur = q.read_output(args.agent, args.lines)
        if cur != before:
            changed = True
        if changed and cur == prev and cur:
            stable += 1
            if stable >= 2:
                print(cur)
                return
        else:
            stable = 0
        prev = cur
        print(".", end="", file=sys.stderr, flush=True)
    print("", file=sys.stderr)
    print(prev)
    sys.exit(f"warning: timeout ({args.timeout}s) — 最後に読めた出力を表示しました")


if __name__ == "__main__":
    main()
