"""Exercise actual proxy sockets and partial framing without a GUI."""
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest

from packaged_webview_support import AgentProxy


def frame(value):
    payload = json.dumps(value).encode()
    return len(payload).to_bytes(4, "big") + payload


def receive(connection):
    data = b""
    while len(data) < 4:
        chunk = connection.recv(4 - len(data))
        if not chunk:
            return data
        data += chunk
    size = int.from_bytes(data, "big")
    while len(data) < 4 + size:
        data += connection.recv(4 + size - len(data))
    return data


class ProxyTests(unittest.TestCase):
    def test_fragmented_requests_drop_only_armed_success_and_never_replay(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as directory:
            root = Path(directory)
            server = socket.socket(socket.AF_UNIX)
            server.bind(str(root / "upstream"))
            server.listen(4)
            server.settimeout(5)
            observed = []
            failures = []

            def serve():
                try:
                    for _ in range(3):
                        connection, _ = server.accept()
                        with connection:
                            connection.settimeout(5)
                            message = json.loads(receive(connection)[4:])
                            observed.append(message)
                            response = frame({"id": message["id"], "status": "success", "value": {}})
                            for part in [response[:2], response[2:7], response[7:]]:
                                connection.sendall(part)
                except Exception as error:
                    failures.append(error)

            worker = threading.Thread(target=serve, daemon=True)
            worker.start()
            proxy = AgentProxy(root / "proxy", root / "upstream")
            try:
                for identifier, operation in [(1, "list-profiles"), (2, "put-kv"), (3, "list-profiles")]:
                    if identifier == 2:
                        self.assertEqual(proxy.arm("/selected"), 0)
                    with socket.socket(socket.AF_UNIX) as client:
                        client.settimeout(5)
                        client.connect(str(root / "proxy"))
                        request = frame({"id": identifier, "operation": {"operation": operation, "path": "/selected"}})
                        client.sendall(request[:1])
                        client.sendall(request[1:8])
                        client.sendall(request[8:])
                        response = receive(client)
                        if identifier == 2:
                            self.assertEqual(response, b"")
                        else:
                            self.assertEqual(json.loads(response[4:])["id"], identifier)
                worker.join(timeout=5)
                self.assertFalse(worker.is_alive())
                self.assertEqual(failures, [])
                self.assertEqual([r["id"] for r in observed], [1, 2, 3])
                self.assertEqual(proxy.snapshot(), {"dropped": 1, "mutations": 1, "errors": []})
            finally:
                proxy.close()
                server.close()
            self.assertFalse((root / "proxy").exists())
            self.assertTrue(all(not w.is_alive() for w in proxy.workers))


if __name__ == "__main__":
    unittest.main()
