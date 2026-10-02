"""Isolated Unix-agent relay for production WebView recovery tests."""
import json
import os
from pathlib import Path
import select
import socket
import threading
import time


class AgentProxy:
    """Forward real IPC, optionally dropping one completed mutation's reply.

    No response is synthesized. Only operation names/path and aggregate counts
    are retained; plaintext payloads are neither logged nor saved as artifacts.
    """
    def __init__(self, path, upstream):
        self.path, self.upstream = Path(path), str(upstream)
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(path))
        os.chmod(path, 0o600)
        self.listener.listen(16)
        self.listener.settimeout(0.1)
        self.stopped = threading.Event()
        self.lock = threading.Lock()
        self.armed = None
        self.dropped = 0
        self.mutations = 0
        self.errors = []
        self.workers = []
        self.connections = []
        self.thread = threading.Thread(target=self._accept, daemon=True)
        self.thread.start()

    def arm(self, path):
        with self.lock:
            assert self.armed is None
            self.armed = path
            return self.mutations

    def snapshot(self):
        with self.lock:
            return {"dropped": self.dropped, "mutations": self.mutations, "errors": list(self.errors)}

    def _accept(self):
        while not self.stopped.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            worker = threading.Thread(target=self._relay, args=(client,), daemon=True)
            self.workers.append(worker)
            worker.start()

    def _relay(self, client):
        upstream = socket.socket(socket.AF_UNIX)
        try:
            upstream.connect(self.upstream)
            with self.lock:
                self.connections.extend([client, upstream])
            buffers = {client: bytearray(), upstream: bytearray()}
            drop_id = None
            deadline = time.monotonic() + 90
            while not self.stopped.is_set() and time.monotonic() < deadline:
                ready, _, _ = select.select([client, upstream], [], [], 0.1)
                for source in ready:
                    chunk = source.recv(65536)
                    if not chunk:
                        return
                    deadline = time.monotonic() + 90
                    data = buffers[source]
                    data.extend(chunk)
                    while len(data) >= 4:
                        length = int.from_bytes(data[:4], "big")
                        if length > 1024 * 1024:
                            raise ValueError("agent frame exceeds production bound")
                        if len(data) < 4 + length:
                            break
                        frame = bytes(data[:4 + length])
                        del data[:4 + length]
                        message = json.loads(frame[4:])
                        if source is client:
                            operation = message.get("operation", {})
                            if isinstance(operation, dict) and operation.get("operation") in {"put-kv", "put-kv-stream"}:
                                with self.lock:
                                    self.mutations += 1
                                    if operation.get("path") == self.armed and self.armed is not None:
                                        drop_id = message["id"]
                                        self.armed = None
                        elif drop_id is not None and message.get("id") == drop_id:
                            # Stream operations can acknowledge headers; the
                            # recovery scenario deliberately uses inline text.
                            if message.get("status") != "success":
                                raise AssertionError("armed mutation did not complete successfully")
                            with self.lock:
                                self.dropped += 1
                            return
                        (upstream if source is client else client).sendall(frame)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Ordinary client cancellation/exit closes read-only requests.
        except Exception as error:
            if not self.stopped.is_set():
                with self.lock:
                    self.errors.append(type(error).__name__ + ": " + str(error))
        finally:
            client.close()
            upstream.close()

    def close(self):
        self.stopped.set()
        self.listener.close()
        with self.lock:
            for connection in self.connections:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        self.thread.join(timeout=2)
        for worker in self.workers:
            worker.join(timeout=2)
        self.path.unlink(missing_ok=True)
