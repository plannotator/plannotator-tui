#!/usr/bin/env python3
"""End-to-end run: the real installed `pi` with the extension, and the real plannotator-tui.

    cargo build -q -p plannotator-tui && python3 -I integrations/pi/e2e.py   (KEEP=1 keeps the temp folder)

Everything lives in a fresh folder under /var/tmp. HOME, Pi's agent and session dirs,
PLANNOTATOR_DATA_DIR, the registry and the XDG dirs all point there. The model is a fake
OpenAI-compatible server inside this script. plannotator-tui runs in a 40x140 pty, and a
small terminal emulator here turns its output into screen text. The script ends every
process it starts and exits 1 if any check fails.
"""

import codecs
import fcntl
import glob
import http.server
import json
import os
import pty
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
EXTENSION = os.environ.get("PI_LINK_EXTENSION") or os.path.join(REPO, "crates/plannotator-tui/src/pi/plannotator-tui.ts")
TUI_BIN = os.environ.get("PLANNOTATOR_TUI_BIN") or os.path.join(REPO, "target/debug/plannotator-tui")
ROWS, COLS = 40, 140

results = []  # (ok, name, evidence)


def check(ok, name, evidence=""):
    results.append((bool(ok), name, evidence))
    print(f"{'ok  ' if ok else 'FAIL'} {name}")
    if evidence:
        for line in str(evidence).splitlines()[:12]:
            print(f"       | {line}")
    return bool(ok)


def until(fn, timeout=15.0, step=0.05):
    end = time.time() + timeout
    while True:
        v = fn()
        if v:
            return v
        if time.time() > end:
            return None
        time.sleep(step)


# ---------------------------------------------------------------------------
# A minimal VT screen: enough of what ratatui/crossterm emit, plus answers to
# the queries crossterm sends at start (cursor position, device attributes, colours).


class Screen:
    def __init__(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.grid = [[" "] * cols for _ in range(rows)]
        self.r = self.c = 0
        self.saved = (0, 0)
        self.state, self.buf = "text", ""
        self.lock = threading.Lock()

    def _scroll(self):
        self.grid.pop(0)
        self.grid.append([" "] * self.cols)
        self.r = self.rows - 1

    def _put(self, ch):
        w = 2 if (unicodedata.east_asian_width(ch) in "WF" or ord(ch) >= 0x1F300) else 1
        if unicodedata.combining(ch):
            return
        if self.c + w > self.cols:
            self.c = 0
            self.r += 1
            if self.r >= self.rows:
                self._scroll()
        self.grid[self.r][self.c] = ch
        if w == 2 and self.c + 1 < self.cols:
            self.grid[self.r][self.c + 1] = ""
        self.c += w

    def feed(self, text):
        replies = []
        with self.lock:
            for ch in text:
                if self.state == "text":
                    if ch == "\x1b":
                        self.state = "esc"
                    elif ch == "\r":
                        self.c = 0
                    elif ch == "\n":
                        self.r += 1
                        if self.r >= self.rows:
                            self._scroll()
                    elif ch == "\b":
                        self.c = max(0, self.c - 1)
                    elif ch == "\t":
                        self.c = min(self.cols - 1, (self.c // 8 + 1) * 8)
                    elif ord(ch) < 0x20 or ch == "\x7f":
                        pass
                    else:
                        self._put(ch)
                elif self.state == "esc":
                    if ch == "[":
                        self.state, self.buf = "csi", ""
                    elif ch == "]":
                        self.state, self.buf = "osc", ""
                    elif ch in "()*+":
                        self.state = "charset"
                    elif ch == "7":
                        self.saved, self.state = (self.r, self.c), "text"
                    elif ch == "8":
                        (self.r, self.c), self.state = self.saved, "text"
                    else:
                        self.state = "text"
                elif self.state == "charset":
                    self.state = "text"
                elif self.state == "csi":
                    if "\x40" <= ch <= "\x7e":
                        replies += self._csi(self.buf, ch)
                        self.state = "text"
                    else:
                        self.buf += ch
                elif self.state == "osc":
                    if ch == "\x07" or (ch == "\\" and self.buf.endswith("\x1b")):
                        replies += self._osc(self.buf.rstrip("\x1b"))
                        self.state = "text"
                    else:
                        self.buf += ch
        return replies

    def _csi(self, params, final):
        private = params[:1] if params[:1] in "?<=>" else ""
        body = params[len(private):]
        nums = [int(p) if p.isdigit() else 0 for p in body.split(";")] if body else []
        n = lambda i, d=1: (nums[i] if i < len(nums) and nums[i] else d)  # noqa: E731
        if final in "Hf":
            self.r, self.c = min(self.rows, n(0)) - 1, min(self.cols, n(1)) - 1
        elif final == "J":
            mode = nums[0] if nums else 0
            if mode in (2, 3):
                self.grid = [[" "] * self.cols for _ in range(self.rows)]
            elif mode == 0:
                self.grid[self.r][self.c:] = [" "] * (self.cols - self.c)
                for row in range(self.r + 1, self.rows):
                    self.grid[row] = [" "] * self.cols
        elif final == "K":
            mode = nums[0] if nums else 0
            if mode == 0:
                self.grid[self.r][self.c:] = [" "] * (self.cols - self.c)
            elif mode == 1:
                self.grid[self.r][: self.c + 1] = [" "] * (self.c + 1)
            else:
                self.grid[self.r] = [" "] * self.cols
        elif final == "X":
            for i in range(self.c, min(self.cols, self.c + n(0))):
                self.grid[self.r][i] = " "
        elif final == "A":
            self.r = max(0, self.r - n(0))
        elif final == "B":
            self.r = min(self.rows - 1, self.r + n(0))
        elif final == "C":
            self.c = min(self.cols - 1, self.c + n(0))
        elif final == "D":
            self.c = max(0, self.c - n(0))
        elif final == "G":
            self.c = min(self.cols, n(0)) - 1
        elif final == "d":
            self.r = min(self.rows, n(0)) - 1
        elif final == "n" and body == "6":
            return [f"\x1b[{self.r + 1};{self.c + 1}R"]
        elif final == "c" and private == "":
            return ["\x1b[?62;22c"]
        return []

    def _osc(self, body):
        if body.startswith("11;?"):
            return ["\x1b]11;rgb:0000/0000/0000\x1b\\"]
        if body.startswith("10;?"):
            return ["\x1b]10;rgb:ffff/ffff/ffff\x1b\\"]
        return []

    def text(self):
        with self.lock:
            lines = ["".join(row).rstrip() for row in self.grid]
        while lines and not lines[-1]:
            lines.pop()
        return "\n".join(lines)


# ---------------------------------------------------------------------------
# Fake model. "CALL-NOTE {json}" as the last user message → a plannotator_tui_note
# call with those args; a tool result as the last message → recorded, answered with
# text; anything else → "ack N". Text replies wait `delay` seconds.


class FakeLLM:
    def __init__(self):
        self.requests, self.tool_results, self.delay = [], [], 0.2
        self.lock = threading.Lock()
        llm = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("content-length") or 0))
                req = json.loads(body or b"{}")
                msgs = req.get("messages", [])
                with llm.lock:
                    llm.requests.append(msgs)
                    n = len(llm.requests)
                last = msgs[-1] if msgs else {}
                text = content_text(last.get("content"))
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.end_headers()

                def chunk(delta, finish=None):
                    obj = {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "fake",
                           "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
                    self.wfile.write(f"data: {json.dumps(obj)}\n\n".encode())

                call = re.search(r"CALL-NOTE (\{.*\})", text, re.S) if last.get("role") == "user" else None
                if last.get("role") == "tool":
                    with llm.lock:
                        llm.tool_results.append(text)
                    time.sleep(llm.delay)
                    chunk({"role": "assistant", "content": f"tool said: {text}"})
                    chunk({}, "stop")
                elif call:
                    chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": f"call_{n}", "type": "function",
                           "function": {"name": "plannotator_tui_note", "arguments": call.group(1)}}]})
                    chunk({}, "tool_calls")
                else:
                    time.sleep(llm.delay)
                    chunk({"role": "assistant", "content": f"ack {n}"})
                    chunk({}, "stop")
                self.wfile.write(b"data: [DONE]\n\n")

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def user_texts(self):
        with self.lock:
            return [content_text(m.get("content")) for req in self.requests for m in req if m.get("role") == "user"]


def content_text(c):
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "".join(p.get("text", "") for p in c if isinstance(p, dict))
    return ""


# ---------------------------------------------------------------------------


def main():
    tmp = tempfile.mkdtemp(prefix="pi-link-e2e-", dir="/var/tmp")
    d = {k: os.path.join(tmp, k) for k in
         ["home", "agent", "sessions", "registry", "data", "work", "xdg/config", "xdg/data", "xdg/state", "xdg/cache", "tmp"]}
    for k, p in d.items():
        if k != "registry":
            os.makedirs(p)
    doc = os.path.join(d["work"], "plan.md")
    with open(doc, "w") as f:
        f.write("# Plan\n\nFirst step.\n\nSecond step.\n")
    doc_real = os.path.realpath(doc)

    llm = FakeLLM()
    with open(os.path.join(d["agent"], "models.json"), "w") as f:
        json.dump({"providers": {"fake": {"baseUrl": f"http://127.0.0.1:{llm.port}/v1", "api": "openai-completions",
                   "apiKey": "x", "models": [{"id": "fake", "name": "fake",
                   "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}}]}}}, f)

    # Only these variables reach the children: no HERDR_*, REX_*, TMUX or real config.
    env = {
        "PATH": os.environ["PATH"], "LANG": "en_US.UTF-8", "HOME": d["home"], "TMPDIR": d["tmp"],
        "PI_CODING_AGENT_DIR": d["agent"], "PI_CODING_AGENT_SESSION_DIR": d["sessions"],
        "PLANNOTATOR_TUI_SESSIONS_DIR": d["registry"], "PLANNOTATOR_DATA_DIR": d["data"],
        "XDG_CONFIG_HOME": d["xdg/config"], "XDG_DATA_HOME": d["xdg/data"],
        "XDG_STATE_HOME": d["xdg/state"], "XDG_CACHE_HOME": d["xdg/cache"],
        "PI_OFFLINE": "1", "PI_SKIP_VERSION_CHECK": "1", "PI_TELEMETRY": "0",
        "PLANNOTATOR_BROWSER": "none",
    }

    pi = None
    tui_pid = None
    master = None
    try:
        # --- Pi ----------------------------------------------------------------
        pi = subprocess.Popen(
            ["pi", "--mode", "rpc", "--provider", "fake", "--model", "fake", "--name", "e2e-pi", "--no-extensions",
             "--no-skills", "--no-context-files", "--offline", "--extension", EXTENSION],
            cwd=d["work"], env={**env, "TERM": "dumb"}, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, bufsize=1)
        events = []

        def read_pi():
            for line in pi.stdout:
                try:
                    events.append(json.loads(line))
                except ValueError:
                    pass
        threading.Thread(target=read_pi, daemon=True).start()

        def rpc(cmd):
            pi.stdin.write(json.dumps(cmd) + "\n")
            pi.stdin.flush()

        def settled_after(mark, timeout=30):
            return until(lambda: any(e.get("type") == "agent_settled" for e in events[mark:]), timeout)

        pi_dir = os.path.join(d["registry"], "pi")
        pi_entry_file = until(lambda: next(iter(glob.glob(os.path.join(pi_dir, "*.json"))), None), 30)
        if not check(pi_entry_file, "Pi registers in the isolated registry", pi_entry_file):
            print(pi.stderr.read() if pi.poll() is not None else "")
            return
        pi_entry = json.load(open(pi_entry_file))

        # --- plannotator-tui in a pty -----------------------------------------
        screen = Screen(ROWS, COLS)
        tui_pid, master = pty.fork()
        if tui_pid == 0:
            try:
                os.chdir(d["work"])
                os.execve(TUI_BIN, [TUI_BIN, doc], {**env, "TERM": "xterm-256color"})
            finally:
                os._exit(127)
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        tui_exit = {}

        def read_tui():
            dec = codecs.getincrementaldecoder("utf-8")("replace")
            while True:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                for reply in screen.feed(dec.decode(data)):
                    try:
                        os.write(master, reply.encode())
                    except OSError:
                        pass
        threading.Thread(target=read_tui, daemon=True).start()

        def tui_alive():
            if "status" in tui_exit:
                return False
            pid, status = os.waitpid(tui_pid, os.WNOHANG)
            if pid:
                tui_exit["status"] = status
                return False
            return True

        def keys(s, pause=0.4):
            os.write(master, s.encode())
            time.sleep(pause)

        def wait_screen(*needles, timeout=10):
            return until(lambda: all(n in screen.text() for n in needles), timeout)

        def footer():
            return "\n".join(screen.text().splitlines()[-3:])

        # Check 1: the TUI lists itself with files = [realpath of the doc].
        check(wait_screen("First step.", "Second step."), "plannotator-tui draws the document", screen.text())
        tui_entry_file = os.path.join(d["registry"], "tui", f"{tui_pid}.json")
        until(lambda: os.path.exists(tui_entry_file), 10)
        try:
            tui_entry = json.load(open(tui_entry_file))
        except (OSError, ValueError) as e:
            tui_entry = {"error": str(e)}
        check(tui_entry.get("files") == [doc_real] and tui_entry.get("host") == "plannotator-tui"
              and tui_entry.get("pid") == tui_pid and len(tui_entry.get("token", "")) == 64,
              "1a. tui/<pid>.json has host, pid, token and files = [realpath of the doc]",
              f"{tui_entry_file}\n{json.dumps(tui_entry)}")
        if os.path.exists(tui_entry_file):
            mode = os.stat(tui_entry_file).st_mode & 0o777
            check(mode == 0o600, "1b. tui entry mode is 0600", oct(mode))

        # Check 2: Pi → TUI note through the real tool and the real listener.
        mark = len(events)
        rpc({"type": "prompt", "message": "CALL-NOTE " + json.dumps(
            {"file": "plan.md", "quote": "Second step.", "note": "Split this into two."})})
        until(lambda: len(llm.tool_results) >= 1, 20)
        result = llm.tool_results[0] if llm.tool_results else "(no tool result)"
        end_ev = until(lambda: next((e for e in events[mark:] if e.get("type") == "tool_execution_end"), None), 5)
        check("Note added" in result and end_ev and not end_ev.get("isError"),
              "2a. the model gets a success result from plannotator_tui_note", result)
        check(wait_screen("Split this into two.", "pi ·"), "2b. the note shows in the rail with 'pi ·'",
              "\n".join(l for l in screen.text().splitlines() if "Split this" in l or "pi ·" in l))
        check(wait_screen("note from pi · e2e-pi"), "2c. footer says 'note from pi · e2e-pi'", footer())
        hits = [p for p in glob.glob(os.path.join(d["data"], "**", "*.json"), recursive=True)
                if "Split this into two." in open(p, encoding="utf-8", errors="replace").read()]
        stored = open(hits[0]).read() if hits else ""
        check(hits, "2d. the note is saved under PLANNOTATOR_DATA_DIR",
              "\n".join(os.path.relpath(h, tmp) for h in hits) + ("\n" + stored[:600] if stored else ""))
        settled_after(mark)

        # Check 3: P → picker → Enter → E sends the review to Pi.
        keys("P")
        check(wait_screen("send to which Pi?", "e2e-pi"), "3a. P opens 'send to which Pi?' listing the session",
              "\n".join(l for l in screen.text().splitlines() if "Pi" in l or "e2e-pi" in l or "back to" in l))
        keys("\r")
        check(wait_screen("Send now goes to pi · e2e-pi"), "3b. Enter picks it: 'Send now goes to pi · e2e-pi'", footer())
        mark = len(events)
        keys("E")
        got = until(lambda: next((t for t in llm.user_texts() if "# Annotations on" in t), None), 15)
        check(got and "Split this into two." in got and "From: pi ·" in got,
              "3c. Pi gets the feedback as a user message ('# Annotations on', the note, 'From: pi ·')", got or "(nothing)")
        check(wait_screen("sent 1 annotation(s) → pi · e2e-pi"), "3d. TUI status says it was sent", footer())
        settled_after(mark)
        transcript = pi_entry.get("transcript") or json.load(open(pi_entry_file)).get("transcript")
        tx = open(transcript).read() if transcript and os.path.exists(transcript) else ""
        check("# Annotations on" in tx and "Split this into two." in tx, "3e. the feedback is in Pi's transcript",
              transcript)

        # Check 4: send while Pi is busy. A second Pi note gives the TUI something new to send;
        # the model then takes 4 s to answer, so Pi is busy when E is pressed.
        llm.delay = 4.0
        mark = len(events)
        n_tools = len(llm.tool_results)
        rpc({"type": "prompt", "message": "CALL-NOTE " + json.dumps(
            {"file": doc_real, "quote": "First step.", "note": "Say who does this."})})
        until(lambda: len(llm.tool_results) > n_tools, 20)
        check(wait_screen("Say who does this."), "4a. a second Pi note arrives",
              llm.tool_results[-1] if llm.tool_results else "")
        busy = until(lambda: json.load(open(pi_entry_file)).get("busy") is True, 5)
        keys("E")
        check(wait_screen("Pi is busy; it reads this next"), "4b. sending while Pi is busy says Pi reads it next",
              f"entry busy={busy}\n{footer()}")
        settled_after(mark, 40)
        # Requests repeat the history, so look for the feedback message itself.
        got = until(lambda: next((t for t in llm.user_texts() if "# Annotations on" in t and "Say who does this." in t), None), 15)
        check(got and "Split this into two." not in got, "4c. Pi reads the queued feedback (only the new note) after the busy turn",
              got or "(nothing)")
        llm.delay = 0.2

        # Check 5: Pi gone. Close its stdin, write a note by keys, press E.
        pi.stdin.close()
        try:
            pi.wait(15)
        except subprocess.TimeoutExpired:
            pass
        check(pi.poll() is not None and not os.path.exists(pi_entry_file), "5a. Pi exits and removes its entry",
              f"exit={pi.poll()} entry_exists={os.path.exists(pi_entry_file)}")
        keys("j")
        keys("c")
        keys("Typed by keys.", 0.6)
        keys("\r", 0.8)
        check(wait_screen("Typed by keys."), "5b. a note composed by keys (j, c, text, Enter) is saved",
              footer())
        keys("E", 1.0)
        check(wait_screen("has closed"), "5c. E reports that Pi has closed", footer())

        # Check 1 (end): quit removes the entry.
        keys("q", 0.8)
        if tui_alive() and ("quit" in screen.text().lower()):
            prompt = footer()
            keys("n", 0.8)
        else:
            prompt = "(no prompt)"
        until(lambda: not tui_alive(), 10)
        check(not tui_alive() and not os.path.exists(tui_entry_file),
              "1c. q (then n at the quit prompt) quits and removes tui/<pid>.json",
              f"prompt:\n{prompt}\nexit={tui_exit.get('status')} entry_exists={os.path.exists(tui_entry_file)}")
    finally:
        if tui_pid:
            try:
                if os.waitpid(tui_pid, os.WNOHANG) == (0, 0):
                    os.kill(tui_pid, signal.SIGKILL)
                    os.waitpid(tui_pid, 0)
            except ChildProcessError:
                pass
        if master is not None:
            try:
                os.close(master)
            except OSError:
                pass
        if pi and pi.poll() is None:
            pi.kill()
            pi.wait()
        llm.server.shutdown()
        failed = [r for r in results if not r[0]]
        if failed or os.environ.get("KEEP"):
            print(f"temp folder kept: {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)
        print(f"\n{'FAIL' if failed else 'PASS'} {len(results) - len(failed)}/{len(results)} checks")
        sys.exit(1 if failed or not results else 0)


if __name__ == "__main__":
    main()
