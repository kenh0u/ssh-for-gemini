#!/usr/bin/env python3
"""OpenSSH ProxyCommand client for ssh-for-gemini (pure HTTP POST/GET, no WebSocket)."""

import http.client
import os
import sys
import threading
import urllib.parse

UA = "Mozilla/5.0 (compatible; Google-Agent; +https://developers.google.com/crawling/docs/crawlers-fetchers/google-agent)"


class HttpClient:
    def __init__(self, parsed: urllib.parse.ParseResult, timeout: float = 30.0):
        self.parsed = parsed
        self.timeout = timeout
        self.conn = None

    def _connect(self):
        if self.conn is not None:
            try:
                self.conn.close()
            except Exception:
                pass
        if self.parsed.scheme == "https":
            self.conn = http.client.HTTPSConnection(
                self.parsed.hostname, self.parsed.port or 443, timeout=self.timeout
            )
        else:
            self.conn = http.client.HTTPConnection(
                self.parsed.hostname, self.parsed.port or 80, timeout=self.timeout
            )

    def request(self, method: str, path: str, body: bytes = b"") -> tuple[int, bytes]:
        full_path = self.parsed.path.rstrip("/") + path
        headers = {
            "User-Agent": UA,
            "Connection": "keep-alive",
            "Content-Length": str(len(body)),
        }
        for attempt in range(2):
            if self.conn is None:
                self._connect()
            try:
                self.conn.request(method, full_path, body=body, headers=headers)
                resp = self.conn.getresponse()
                data = resp.read()
                return resp.status, data
            except (http.client.HTTPException, OSError):
                self.conn = None
                if attempt == 1:
                    raise


def main():
    if len(sys.argv) != 2:
        sys.stderr.write("Usage: http-ssh-proxy.py https://ssh-bridge.example.com\n")
        sys.exit(2)

    parsed = urllib.parse.urlparse(sys.argv[1])
    tx_client = HttpClient(parsed, timeout=20.0)
    rx_client = HttpClient(parsed, timeout=30.0)

    status, sid_bytes = tx_client.request("POST", "/open")
    if status != 200:
        sys.stderr.write(f"http-ssh-proxy: /open failed ({status}): {sid_bytes.decode(errors='replace')}\n")
        sys.exit(1)
    sid = sid_bytes.decode().strip()

    stop = threading.Event()

    def tx_loop():
        try:
            while not stop.is_set():
                data = os.read(0, 16384)
                if not data:
                    break
                st, _ = tx_client.request("POST", f"/tx/{sid}", data)
                if st not in (200, 204):
                    break
        except Exception:
            pass
        finally:
            stop.set()

    t = threading.Thread(target=tx_loop, daemon=True)
    t.start()

    try:
        while not stop.is_set():
            st, data = rx_client.request("GET", f"/rx/{sid}")
            if st == 200 and data:
                view = memoryview(data)
                written = 0
                while written < len(data):
                    written += os.write(1, view[written:])
            elif st == 204:
                continue
            else:
                break
    except Exception:
        pass
    finally:
        stop.set()
        try:
            HttpClient(parsed, timeout=5.0).request("POST", f"/close/{sid}")
        except Exception:
            pass


if __name__ == "__main__":
    main()
