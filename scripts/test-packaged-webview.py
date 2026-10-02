#!/usr/bin/env python3
"""Exercise production Linux WebView UI against an isolated local FOKS server.

Run under xvfb-run with tauri-driver, WebKitWebDriver, openbox and xdotool.
The Rust fixture seeds synthetic private-file credentials; no native keyring,
normal user state, remote service, or production-only app hook is used.
"""
import argparse
import base64
from contextlib import suppress
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

from packaged_webview_support import AgentProxy


class WebDriverError(RuntimeError):
    pass


def request(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        "http://127.0.0.1:4444" + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=40) as response:
            value = json.load(response)["value"]
    except urllib.error.HTTPError as error:
        raise WebDriverError(error.read().decode()) from error
    if isinstance(value, dict) and "error" in value:
        raise WebDriverError(value)
    return value


def eventually(check, timeout=40):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (OSError, urllib.error.URLError, WebDriverError) as error:
            last = error
        time.sleep(0.1)
    raise TimeoutError(f"WebView readiness deadline exceeded; last error: {last}")


def stop(process):
    if process is not None:
        with suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            with suppress(ProcessLookupError):
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


class Fixture:
    def __init__(self, binary, state, environment, log):
        self.process = subprocess.Popen(
            [str(binary), str(state)], env=environment, text=True, bufsize=1,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log,
            start_new_session=True,
        )
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        for line in self.process.stdout:
            self.lines.put(json.loads(line))
        self.lines.put({"fixture_exited": self.process.poll()})

    def receive(self):
        value = self.lines.get(timeout=60)
        assert "fixture_exited" not in value, value
        return value

    def close(self):
        # EOF/stop lets Rust drop the server and its own temporary tree first.
        try:
            if self.process.poll() is None:
                self.process.stdin.write('{"command":"stop"}\n')
                self.process.stdin.flush()
                self.process.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            stop(self.process)
        finally:
            self.process.stdin.close()
            self.reader.join(timeout=2)
            self.process.stdout.close()

    def call(self, command):
        self.process.stdin.write(json.dumps({"command": command}) + "\n")
        self.process.stdin.flush()
        return self.receive()


class WebView:
    def __init__(self, application, environment, log, artifacts, phase):
        self.artifacts, self.phase = artifacts, phase
        self.session = None
        self.driver = subprocess.Popen(
            ["tauri-driver"], env=environment, stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            eventually(lambda: request("GET", "/status"))
            created = request("POST", "/session", {"capabilities": {"alwaysMatch": {
                "tauri:options": {"application": str(application)},
            }}})
            self.session = "/session/" + created["sessionId"]
            request("POST", self.session + "/timeouts", {"script": 30000})
            eventually(lambda: self.execute("return !!document.querySelector('#root')?.textContent.trim()"))
            assert self.execute("return location.protocol !== 'http:' || location.hostname === 'tauri.localhost'")
            assert not self.execute("return !!document.querySelector('#app-boot-error-title')")
        except BaseException:
            self.close()
            raise

    def execute(self, script):
        return request("POST", self.session + "/execute/sync", {"script": script, "args": []})

    def invoke_smoke(self, command):
        # Existing bootstrap/boundary checks only. Authenticated flows below
        # exclusively use WebDriver element interactions and native picker keys.
        return request("POST", self.session + "/execute/async", {
            "script": """const done = arguments[arguments.length - 1];
            window.__TAURI_INTERNALS__.invoke(arguments[0]).then(
              value => done({ok: true, value}), error => done({ok: false, error}));""",
            "args": [command],
        })

    def element(self, xpath):
        values = request("POST", self.session + "/elements", {"using": "xpath", "value": xpath})
        for value in values:
            identifier = value["element-6066-11e4-a52e-4f735466cecf"]
            if request("GET", self.session + "/element/" + identifier + "/displayed"):
                return identifier
        return None

    def click(self, xpath):
        identifier = eventually(lambda: self.element(xpath))
        request("POST", self.session + "/element/" + identifier + "/click", {})

    def button(self, label):
        self.click(f"//button[normalize-space(.)='{label}']")

    def fill(self, value):
        identifier = eventually(lambda: self.element("//textarea[@aria-label='Contents']"))
        request("POST", self.session + "/element/" + identifier + "/clear", {})
        request("POST", self.session + "/element/" + identifier + "/value", {"text": value})

    def select(self, name, wait=True):
        self.click(f"//*[@role='button'][.//span[@title='{name}']]")
        if wait:
            eventually(lambda: self.element(f"//aside[@aria-label='Details for {name}']"))

    def screenshot(self, name):
        (self.artifacts / name).write_bytes(base64.b64decode(request("GET", self.session + "/screenshot")))

    def close(self):
        if self.session:
            with suppress(Exception):
                self.screenshot(self.phase + ".png")
            with suppress(Exception):
                request("DELETE", self.session)
            self.session = None
        stop(self.driver)


def launch_agent(binary, state, socket, environment, log):
    agent = subprocess.Popen(
        [str(binary), "--state-dir", str(state), "--socket", str(socket),
         "--scheduler-poll-seconds", "3600", "--compatibility-poll-seconds", "3600"],
        env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
    )
    try:
        eventually(lambda: socket.exists() and agent.poll() is None)
    except BaseException:
        stop(agent)
        raise
    return agent


def native_file_selection(path):
    def dialog():
        result = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--name", "^(Open|Open File|Choose.*)$"],
            capture_output=True, text=True, timeout=5,
        )
        return result.stdout.splitlines()[-1] if result.returncode == 0 and result.stdout.strip() else None
    window = eventually(dialog)
    subprocess.run(["xdotool", "windowactivate", "--sync", window], check=True, timeout=5)
    subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+l"], check=True, timeout=5)
    subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "1", str(path)], check=True, timeout=10)
    subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], check=True, timeout=5)
    eventually(lambda: not dialog())


def entry(snapshot, path):
    return next(row for row in snapshot["entries"] if row["path"] == path)


def authenticated_flow(ui, fixture, proxy, replacement, artifacts):
    ui.button("All items")
    ui.select("webview-note.txt")
    ui.button("Edit")
    ui.fill("discard this synthetic draft")
    ui.select("webview-file.bin", wait=False)
    eventually(lambda: ui.element("//*[@role='alertdialog'][.//*[normalize-space(.)='Discard changes?']]"))
    ui.click("//*[@role='alertdialog']//button[normalize-space(.)='Cancel']")
    assert ui.execute("return document.querySelector('textarea[aria-label=Contents]')?.value") == "discard this synthetic draft"
    ui.select("webview-file.bin", wait=False)
    ui.button("Discard")
    assert fixture.call("inspect")["note"] == "initial text"

    ui.select("webview-note.txt")
    ui.button("Edit")
    ui.fill("edited through production WebView")
    ui.button("Save changes")
    eventually(lambda: not ui.element("//textarea[@aria-label='Contents']"))
    saved = fixture.call("inspect")
    assert saved["note"] == "edited through production WebView", saved
    assert entry(saved, "/webview-note.txt")["version"] == 2, saved
    ui.screenshot("authenticated-edit.png")

    ui.select("webview-file.bin")
    ui.button("Replace")
    ui.button("Choose file…")
    native_file_selection(replacement)
    eventually(lambda: not ui.element("//*[@role='dialog'][.//*[normalize-space(.)='Replace webview-file.bin']]"))
    replaced = fixture.call("inspect")
    assert replaced["file_bytes"] == 8192 and replaced["file_all_replacement"], replaced
    assert entry(replaced, "/webview-file.bin")["version"] == 2, replaced

    ui.select("webview-note.txt")
    ui.button("Edit")
    ui.fill("recovered without resubmission")
    before = fixture.call("inspect")
    fault = fixture.call("arm-lost-reply")
    submitted = proxy.arm("/webview-note.txt")
    ui.button("Save changes")
    eventually(lambda: proxy.snapshot()["dropped"] == 1)
    eventually(lambda: ui.execute("return !Array.from(document.querySelectorAll('button')).some(button => button.textContent.trim() === 'Saving…')"))
    ui.screenshot("interrupted-outcome.png")
    assert proxy.snapshot()["mutations"] == submitted + 1
    # The IPC reply was dropped after success: the UI must recover by reads.
    # Reload also exercises startup/catalog recovery without replaying the edit.
    request("POST", ui.session + "/refresh", {})
    ui.button("All items")
    ui.select("webview-note.txt")
    ui.button("Edit")
    eventually(lambda: ui.element("//textarea[@aria-label='Contents']"))
    assert ui.execute("return document.querySelector('textarea[aria-label=Contents]')?.value") == "recovered without resubmission"
    ui.button("Cancel")
    after = fixture.call("inspect")
    traffic = proxy.snapshot()
    assert after["fault_hits"] == fault["hits_before"] + 1, after
    assert after["namespace_journals"] == before["namespace_journals"] + 1, after
    assert entry(after, "/webview-note.txt")["version"] == entry(before, "/webview-note.txt")["version"] + 1, after
    assert traffic == {"dropped": 1, "mutations": submitted + 1, "errors": []}, traffic
    (artifacts / "authenticated-results.json").write_text(json.dumps({
        "editing": "passed", "discard_dialog": "passed", "native_file_picker": "passed",
        "post_commit_disconnect": "passed", "reload_recovery": "passed",
        "duplicate_mutation_requests": 0, "proxy": traffic,
        "server_fault_hits": after["fault_hits"],
    }, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("application", type=Path)
    parser.add_argument("--agent", type=Path, default=Path("target/release/foks-agent"))
    parser.add_argument("--fixture", type=Path, default=Path("target/debug/examples/webview_fixture"))
    parser.add_argument("--artifacts", type=Path, default=Path("target/webview-test"))
    args = parser.parse_args()
    application, agent_binary, fixture_binary = [p.resolve() for p in (args.application, args.agent, args.fixture)]
    if not all(p.is_file() for p in (application, agent_binary, fixture_binary)):
        parser.error("build the production application/agent and webview_fixture example first")
    args.artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="foks-webview-", dir="/tmp") as temporary:
        root = Path(temporary)
        environment = dict(os.environ)
        for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            environment[name] = str(root / name.lower())
        agent = window_manager = fixture = proxy = ui = None
        with (args.artifacts / "agent.log").open("w") as agent_log, \
                (args.artifacts / "driver.log").open("w") as driver_log, \
                (args.artifacts / "fixture.log").open("w") as fixture_log, \
                (args.artifacts / "window-manager.log").open("w") as wm_log:
            try:
                window_manager = subprocess.Popen(["openbox"], env=environment, stdout=wm_log, stderr=subprocess.STDOUT, start_new_session=True)
                socket = root / "bootstrap.sock"
                environment["FOKS_AGENT_SOCKET"] = str(socket)
                agent = launch_agent(agent_binary, root / "bootstrap", socket, environment, agent_log)
                ui = WebView(application, environment, driver_log, args.artifacts, "bootstrap")
                status = ui.invoke_smoke("agent_status")
                assert status["ok"] and status["value"]["state"] == "bootstrap", status
                lock = ui.invoke_smoke("app_lock_state")
                assert lock["ok"] and isinstance(lock["value"]["locked"], bool), lock
                stop(agent)
                agent = None
                assert not ui.invoke_smoke("agent_status")["ok"]
                ui.close()
                ui = None

                state = root / "authenticated"
                fixture = Fixture(fixture_binary, state, environment, fixture_log)
                assert fixture.receive() == {"ready": True}
                actual_socket, socket = root / "real.sock", root / "proxy.sock"
                environment["FOKS_AGENT_SOCKET"] = str(socket)
                agent = launch_agent(agent_binary, state, actual_socket, environment, agent_log)
                proxy = AgentProxy(socket, actual_socket)
                replacement = root / "replacement.bin"
                replacement.write_bytes(b"b" * 8192)
                ui = WebView(application, environment, driver_log, args.artifacts, "authenticated-final")
                authenticated_flow(ui, fixture, proxy, replacement, args.artifacts)
                print("Production WebView bootstrap, authenticated edits/dialogs/file picker and interrupted-mutation recovery passed")
            finally:
                if ui:
                    ui.close()
                if proxy:
                    (args.artifacts / "proxy-results.json").write_text(json.dumps(proxy.snapshot(), indent=2) + "\n")
                    proxy.close()
                stop(agent)
                if fixture:
                    fixture.close()
                stop(window_manager)


if __name__ == "__main__":
    main()
