#!/usr/bin/env python3
"""Exercise workspace onboarding, collection management and environment isolation in a real PTY."""
import fcntl
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
import time
from terminal_screen import Screen

root = Path(__file__).resolve().parent.parent

class Session:
    def __init__(self, config):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        self.process = subprocess.Popen([str(root / "target/debug/postrust")], stdin=slave, stdout=slave, stderr=slave,
            env={**os.environ, "TERM": "xterm-256color", "POSTRUST_CLIPBOARD": "internal", "XDG_CONFIG_HOME": config}, start_new_session=True)
        os.close(slave)
        self.screen = Screen()

    def send(self, data):
        os.write(self.master, data)

    def paste(self, text):
        self.send(b"\x1b[200~" + text.encode() + b"\x1b[201~")

    def pump(self, timeout=0.1):
        ready, _, _ = select.select([self.master], [], [], timeout)
        if ready:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                return
            self.screen.feed(chunk)
            if b"\x1b[6n" in chunk:
                self.send(b"\x1b[1;1R")

    def wait(self, text):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            self.pump()
            if text in self.screen.text:
                return
            if self.process.poll() is not None:
                break
        raise AssertionError(f"Missing {text!r}:\n{self.screen.text}")

    def settle(self):
        deadline=time.monotonic()+0.3
        while time.monotonic()<deadline:
            ready,_,_=select.select([self.master],[],[],0.03)
            if not ready:
                return
            self.pump(0)

    def wait_absent(self,text):
        deadline=time.monotonic()+8
        while time.monotonic()<deadline:
            self.pump()
            if text not in self.screen.text:
                self.settle()
                return
        raise AssertionError(f"Still showing {text!r}:\n{self.screen.text}")

    def click(self, text):
        self.wait(text)
        self.settle()
        for y, line in enumerate(self.screen.text.splitlines()):
            if text in line:
                x = line.index(text) + 1
                self.send(f"\x1b[<0;{x};{y+1}M\x1b[<0;{x};{y+1}m".encode())
                return
        raise AssertionError(f"Missing mouse target {text!r}:\n{self.screen.text}")

    def click_list_item(self, title, index):
        self.wait(title)
        for y,line in enumerate(self.screen.text.splitlines()):
            if title in line:
                x=line.index(title)
                self.send(f"\x1b[<0;{x+1};{y+2+index}M\x1b[<0;{x+1};{y+2+index}m".encode())
                return

    def save_snapshot(self, name):
        self.settle()
        directory = root / "target/ui-review"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / name).write_text(self.screen.text)

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            self.process.wait(timeout=5)
        os.close(self.master)

with tempfile.TemporaryDirectory(prefix="postrust-workspaces-") as config:
    session = Session(config)
    try:
        session.wait("Workspace name")
        session.save_snapshot("launcher-120x40.txt")
        session.paste("Alpha")
        session.send(b"\r\r\r")
        session.wait("No request selected")
        session.click("F3 Environment")
        session.wait("Workspace environment")
        session.send(b"\x01")
        session.paste(json.dumps({"base_url": "https://alpha.example", "token": "alpha-only"}))
        session.click("Apply & save")
        session.wait("Saved ")
        session.click("+ Folder")
        session.wait("New collection name")
        session.paste("Users")
        session.send(b"\r")
        session.wait("Users (0)")
        session.click("+ Request")
        session.wait("Request name")
        session.paste("List users")
        session.send(b"\t")
        session.wait("List users")
        session.send(b"\x07")  # Ctrl+G
        session.wait("New collection name")
        session.paste("Health")
        session.send(b"\r")
        session.wait("Health (0)")
        session.click("Collection · move")
        session.wait("Move request to collection")
        session.click_list_item("Move request to collection",2)
        session.click("Apply")
        session.wait("Health (1)")
        session.send(b"\x1b[18~")  # F7 rename collection selected by current request
        session.wait("Rename collection")
        session.paste("Status")
        session.send(b"\r")
        session.wait("Status (1)")
        # Collapsing and expanding a folder must hide and reveal its request.
        session.click("Status (1)")
        session.pump()
        session.click("Status (1)")
        session.wait("GET List users")
        session.save_snapshot("tree-120x40.txt")
        session.send(b"\x13")
        session.wait("Saved ")
        alpha = Path(config, "postrust/workspaces/alpha.json")
        data = json.loads(alpha.read_text())
        assert data["requests"][0]["collection"] == "Status", data
        assert "Users" in data["collections"] and "Status" in data["collections"]
        assert data["environment"]["token"] == "alpha-only"
        session.click("F2 Workspaces")
        session.wait("Enter a workspace")
        session.click("Create new")
        session.wait("Workspace name")
        session.send(b"\x01")
        session.paste("Beta")
        session.send(b"\r\r\r")
        session.wait("No request selected")
        session.click("F3 Environment")
        session.wait("Workspace environment")
        assert "alpha-only" not in session.screen.text, session.screen.text
        session.send(b"\x01")
        session.paste(json.dumps({"base_url": "https://beta.example", "token": "beta-only"}))
        session.send(b"\x13")
        session.wait("Saved ")
        beta = Path(config, "postrust/workspaces/beta.json")
        assert json.loads(beta.read_text())["environment"]["token"] == "beta-only"
        session.send(b"\x1b[12~")  # F2 workspace chooser
        session.wait("Enter a workspace")
        session.click("Alpha")
        session.wait("Status (1)")
        session.click("F3 Environment")
        session.wait("alpha-only")
        assert "beta-only" not in session.screen.text
        session.send(b"\x1b")
        session.wait_absent("Workspace environment")
        session.wait("Request tree")
        # Deleting a collection shows an explicit confirmation and preserves other collections.
        session.click("Status (1)")
        session.click("Delete")
        session.wait("Delete collection Status?")
        session.click("Cancel")
        session.wait("Status (1)")
        session.click("Delete")
        session.wait("Delete collection Status?")
        session.send(b"\r")
        session.wait("No request selected")
        session.send(b"\x13\x11")
        session.process.wait(timeout=5)
        assert session.process.returncode == 0
        data = json.loads(alpha.read_text())
        assert "Status" not in data["collections"]
        assert "Users" in data["collections"]
        assert data["requests"] == []
        assert data["environment"]["token"] == "alpha-only"
        print("PASS: workspace chooser, create/reopen two workspaces, isolated environments, collection tree expand/collapse, create/rename/move/delete, mouse dialogs, empty folders, persistence, clean quit")
    finally:
        session.close()
