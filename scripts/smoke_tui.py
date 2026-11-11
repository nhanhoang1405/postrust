#!/usr/bin/env python3
"""Real PTY + local HTTP server smoke test. No third party Python packages."""
import fcntl
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
from terminal_screen import Screen

root = Path(__file__).resolve().parent.parent
received = []

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode()
        received.append((self.path, dict(self.headers), body))
        payload = json.dumps({"message": "postrust-smoke-ok", "body": body}).encode()
        self.send_response(201)
        self.send_header("Content-Type", "application/json")
        self.send_header("X-Smoke", f"headers-ok-{len(received)}")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_):
        pass

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory(prefix="postrust-smoke-") as config:
    workspace = Path(config, "workspace.json")
    workspace.write_text(json.dumps({
        "version": 2,
        "name": "HTTP smoke", "collections": ["Smoke"],
        "requests": [{"name": "Echo", "collection": "Smoke", "method": "POST", "url": "{{base_url}}/echo",
                      "params": "q=hello world", "headers": "Content-Type: application/json\nX-Profile: {{profile}}",
                      "body": '{"message":"{{profile}}"}', "bearer": "{{token}}"}],
        "environment": {"base_url": f"http://127.0.0.1:{server.server_port}", "profile": "local", "token": "smoke-token"},
        "history": [],
    }))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    process = subprocess.Popen(
        [str(root / "target/debug/postrust"), "--workspace", str(workspace)],
        stdin=slave, stdout=slave, stderr=slave,
        env={**os.environ, "TERM": "xterm-256color", "POSTRUST_CLIPBOARD": "internal", "NO_PROXY": "127.0.0.1", "XDG_CONFIG_HOME": str(Path(config, "config"))},
        start_new_session=True,
    )
    os.close(slave)
    screen = Screen()

    def wait_for(text, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.1)
            if ready:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                screen.feed(chunk)
                if b"\x1b[6n" in chunk:
                    os.write(master, b"\x1b[1;1R")
            if text in screen.text:
                return
            if process.poll() is not None:
                break
        raise AssertionError(f"Did not see {text!r}; process={process.poll()}; screen:\n{screen.text}")

    def click_label(label):
        for y, line in enumerate(screen.text.splitlines()):
            if label in line:
                x = line.index(label) + 1
                os.write(master, f"\x1b[<0;{x};{y+1}M\x1b[<0;{x};{y+1}m".encode())
                return
        raise AssertionError(f"Missing mouse target {label!r}")

    def drag_text(text):
        wait_for(text)
        for y,line in enumerate(screen.text.splitlines()):
            if text in line:
                start=line.index(text)+1
                end=start+len(text)
                os.write(master,f"\x1b[<0;{start};{y+1}M\x1b[<32;{end};{y+1}M\x1b[<0;{end};{y+1}m".encode())
                return
        raise AssertionError(f"Missing selection text {text!r}")

    try:
        wait_for("Collections")
        os.write(master, b"\x1b[15~")  # F5
        wait_for("postrust-smoke-ok")
        assert received[0][0] == "/echo?q=hello+world", received
        assert received[0][1]["authorization"] == "Bearer smoke-token", received
        assert received[0][1]["x-profile"] == "local", received
        assert json.loads(received[0][2]) == {"message": "local"}
        os.write(master, b"\x1b[17~")  # F6
        wait_for("headers-ok")
        click_label("Body")
        wait_for("raw text / JSON")
        click_label("raw text / JSON")
        # Place cursor in the body, copy all, replace, paste back, then undo/redo.
        os.write(master, b"\x01\x03")
        wait_for("Copied to internal clipboard")
        os.write(master, b"\x01\x1b[200~temporary\x1b[201~\x01\x16\x1a\x19")
        wait_for("{{profile}}")
        click_label("Send F5")
        wait_for("headers-ok-2")
        assert len(received) == 2, received
        assert json.loads(received[1][2]) == {"message": "local"}, received
        # Select part of output headers, copy it, then send the pasted text as a body.
        drag_text("headers-ok-2")
        os.write(master,b"\x03")
        wait_for("Copied to internal clipboard")
        os.write(master,b"\x1b3\x01\x16")  # Alt+3 Body, select all, paste output selection
        click_label("Send F5")
        wait_for("headers-ok-3")
        assert received[2][2] == "headers-ok-2", received
        os.write(master,b"\x1b3\x1a\x1a")  # Restore original request body.
        os.write(master, b"\x1b[13~")  # F3 workspace environment
        wait_for("Workspace environment")
        # Replace JSON to exercise real profile editing and Unicode bracketed paste.
        env = {"base_url": f"http://127.0.0.1:{server.server_port}", "profile": "世界", "token": "smoke-token"}
        os.write(master, b"\x01\x1b[200~" + json.dumps(env, ensure_ascii=False).encode() + b"\x1b[201~\x13")
        wait_for("Saved ")
        saved = json.loads(workspace.read_text())
        assert saved["environment"]["profile"] == "世界"
        assert len(saved["history"]) == 3
        assert saved["requests"][0]["body"] == '{"message":"{{profile}}"}'
        assert workspace.stat().st_mode & 0o777 == 0o600
        os.write(master, b"\x1b[19~")  # F8 history
        wait_for("Enter restores")
        os.write(master, b"\r")
        wait_for("Restored history")
        os.write(master, b"\x11")
        wait_for("Unsaved changes")
        os.write(master, b"\x1b")  # cancel quit
        wait_for("Collections")
        os.write(master, b"\x13\x11")
        process.wait(timeout=5)
        assert process.returncode == 0
        assert len(json.loads(workspace.read_text())["requests"]) == 2
        print("PASS: real PTY, local POST, params, headers, bearer auth, variables, response tabs, mouse send/editor/output selection, Alt+3, Ctrl+C/V, undo/redo, Unicode environments, private save, history restore, quit guard, terminal cleanup")
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        os.close(master)
        server.shutdown()
