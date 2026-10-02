#!/usr/bin/env python3
"""Exercise a production Linux WebView through tauri-driver (no test app hooks).

Run under xvfb-run with tauri-driver and WebKitWebDriver on PATH. All agent
state is temporary; no account, native credential store, or remote host is used.
"""
import argparse
import base64
from contextlib import suppress
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def request(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        "http://127.0.0.1:4444" + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=40) as response:
        value = json.load(response)["value"]
    if isinstance(value, dict) and "error" in value:
        raise RuntimeError(value)
    return value


def eventually(check, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise TimeoutError("WebView readiness deadline exceeded")


def stop(process):
    if process is not None:
        # The driver also owns WebKitWebDriver and the launched application.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("application", type=Path)
    parser.add_argument("--agent", type=Path, default=Path("target/release/foks-agent"))
    parser.add_argument("--artifacts", type=Path, default=Path("target/webview-test"))
    args = parser.parse_args()
    application, agent_binary = args.application.resolve(), args.agent.resolve()
    if not application.is_file() or not agent_binary.is_file():
        parser.error("build the production application and agent first")
    args.artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="foks-webview-", dir="/tmp") as temporary:
        state = Path(temporary)
        socket = state / "agent.sock"
        environment = dict(os.environ, FOKS_AGENT_SOCKET=str(socket))
        for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            environment[name] = str(state / name.lower())
        agent = driver = None
        session = None
        with (args.artifacts / "agent.log").open("w") as agent_log, \
                (args.artifacts / "driver.log").open("w") as driver_log:
            try:
                agent = subprocess.Popen(
                    [str(agent_binary), "--state-dir", str(state / "state"), "--socket", str(socket)],
                    env=environment, stdout=agent_log, stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
                eventually(lambda: socket.exists() and agent.poll() is None)
                driver = subprocess.Popen(
                    ["tauri-driver"], env=environment, stdout=driver_log,
                    stderr=subprocess.STDOUT, start_new_session=True,
                )
                eventually(lambda: request("GET", "/status"))
                created = request("POST", "/session", {"capabilities": {"alwaysMatch": {
                    "tauri:options": {"application": str(application)},
                }}})
                session = "/session/" + created["sessionId"]
                request("POST", session + "/timeouts", {"script": 30000})

                def execute(script):
                    return request("POST", session + "/execute/sync", {"script": script, "args": []})

                def invoke(command):
                    return request("POST", session + "/execute/async", {
                        "script": """const done = arguments[arguments.length - 1];
                        window.__TAURI_INTERNALS__.invoke(arguments[0]).then(
                          value => done({ok: true, value}),
                          error => done({ok: false, error}));""", "args": [command],
                    })

                eventually(lambda: execute("return !!document.querySelector('#root')?.textContent.trim()"))
                assert execute("return location.protocol !== 'http:' || location.hostname === 'tauri.localhost'")
                status = invoke("agent_status")
                assert status["ok"] and status["value"]["state"] == "bootstrap", status
                lock = invoke("app_lock_state")
                assert lock["ok"] and isinstance(lock["value"]["locked"], bool), lock
                assert not execute("return !!document.querySelector('#app-boot-error-title')")
                # Confirm the renderer/native boundary reports actual agent loss.
                stop(agent)
                agent = None
                disconnected = invoke("agent_status")
                assert not disconnected["ok"], disconnected
                print("Production WebView rendered; native IPC and agent-disconnect checks passed")
            finally:
                if session:
                    with suppress(Exception):
                        screenshot = request("GET", session + "/screenshot")
                        (args.artifacts / "webview.png").write_bytes(base64.b64decode(screenshot))
                    with suppress(Exception):
                        request("DELETE", session)
                stop(driver)
                stop(agent)


if __name__ == "__main__":
    main()
