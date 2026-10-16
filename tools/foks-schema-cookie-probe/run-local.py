#!/usr/bin/env python3
"""Fixed SQL schema-cookie authorizer probe on local celld; no kernel/route activation."""
import argparse
import hashlib
import json
import os
import pathlib
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request

import boto3
from botocore.config import Config


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def result_ok(status, result):
    return status == 200 and isinstance(result, dict) and result.get("passed") is True and result.get("final_table_absent") is True


def bounded_json(response):
    body = response.read(8193)
    if len(body) > 8192:
        raise RuntimeError("probe response bound")
    return json.loads(body)


def main():
    parser = argparse.ArgumentParser()
    for name in ("celld", "moto", "esbuild", "output"):
        parser.add_argument("--" + name, type=pathlib.Path, required=True)
    args = parser.parse_args()
    source = pathlib.Path(__file__).resolve().parent
    inputs = [args.celld, args.moto, args.esbuild, source / "probe.mjs", source / "worker.mjs", pathlib.Path(__file__)]
    if any(not item.is_file() for item in inputs):
        parser.error("missing immutable input")
    before = {str(item.resolve()): digest(item) for item in inputs}
    args.output.mkdir(mode=0o700, exist_ok=False)
    root = args.output.resolve()
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "TMPDIR": str(root),
        "AWS_ACCESS_KEY_ID": "local-cookie", "AWS_SECRET_ACCESS_KEY": "local-cookie-dummy-secret",
        "AWS_DEFAULT_REGION": "us-east-1", "AWS_REGION": "us-east-1",
        "AWS_CONFIG_FILE": "/dev/null", "AWS_SHARED_CREDENTIALS_FILE": "/dev/null",
        "AWS_EC2_METADATA_DISABLED": "true", "CELLD_OTEL": "0", "NO_COLOR": "1",
    }
    children, logs = [], []
    passed, result, error = False, None, None
    token = secrets.token_urlsafe(32)

    def port():
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def start(command, name, extra=None):
        log = (root / (name + ".log")).open("x")
        logs.append(log)
        child = subprocess.Popen(command, cwd=root, env=dict(env, **(extra or {})), stdout=log, stderr=subprocess.STDOUT)
        children.append(child)
        return child

    def request(url, credential):
        req = urllib.request.Request(url, data=b"", headers={"authorization": "Bearer " + credential})
        try:
            return urllib.request.urlopen(req, timeout=15)
        except urllib.error.HTTPError as rejected:
            return rejected

    try:
        endpoint = "http://127.0.0.1:" + str(port())
        moto = start([str(args.moto.resolve()), "-H", "127.0.0.1", "-p", endpoint.rsplit(":", 1)[1]], "moto")
        s3 = boto3.client("s3", endpoint_url=endpoint, region_name="us-east-1", aws_access_key_id=env["AWS_ACCESS_KEY_ID"], aws_secret_access_key=env["AWS_SECRET_ACCESS_KEY"], config=Config(retries={"max_attempts": 0}, connect_timeout=1, read_timeout=2))
        until = time.monotonic() + 20
        while True:
            try:
                s3.list_buckets()
                break
            except Exception:
                if moto.poll() is not None or time.monotonic() > until:
                    raise
                time.sleep(.1)
        bucket = "foks-schema-cookie-probe"
        s3.create_bucket(Bucket=bucket)
        project = root / "worker"
        project.mkdir(mode=0o700)
        for name in ("probe.mjs", "worker.mjs"):
            data = (source / name).read_bytes()
            (project / name).write_bytes(data)
            if digest(project / name) != before[str((source / name).resolve())]:
                raise RuntimeError("source copy changed")
        config = {
            "name": "foks-schema-cookie-probe", "main": "worker.mjs", "compatibility_date": "2026-01-01",
            "vars": {"PROBE_TOKEN": token},
            "durable_objects": {"bindings": [{"name": "COOKIE_PROBE", "class_name": "SchemaCookieProbe"}]},
            "migrations": [{"tag": "v1", "new_sqlite_classes": ["SchemaCookieProbe"]}],
        }
        config_path = project / "wrangler.jsonc"
        config_path.write_text(json.dumps(config))
        config_path.chmod(0o600)
        common = ["--bucket", "s3://" + bucket, "--endpoint", endpoint, "--region", "us-east-1"]
        deploy = subprocess.run([str(args.celld.resolve()), "deploy", str(project)] + common, cwd=root, env=dict(env, CELLD_ESBUILD=str(args.esbuild.resolve())), capture_output=True, text=True, timeout=45)
        (root / "deploy.log").write_text((deploy.stdout + deploy.stderr).replace(token, "[redacted]"))
        if deploy.returncode:
            raise RuntimeError("deployment failed")
        public, internal = port(), port()
        watch = root / "node"
        watch.mkdir()
        node = start([str(args.celld.resolve())] + common + ["--listen", f"127.0.0.1:{public}", "--internal-listen", f"127.0.0.1:{internal}", "--advertise", f"127.0.0.1:{internal}"], "celld", {"CELLD_NODE": "cookie-probe", "CELLD_WATCH": str(watch), "CELLD_READY_FLEET_GATE_MS": "0", "CELLD_DURABILITY": "bucket", "CELLD_MAX_RESIDENT_CELLS": "32", "CELLD_V8_HEAP_LIMIT_MB": "64", "CELLD_TOKIO_THREADS": "2"})
        url = f"http://127.0.0.1:{public}/"
        until = time.monotonic() + 30
        while True:
            try:
                with urllib.request.urlopen(url + ".well-known/celld/health", timeout=1) as response:
                    json.load(response)
                break
            except (OSError, ValueError):
                if node.poll() is not None or time.monotonic() > until:
                    raise
                time.sleep(.1)
        with request(url, "wrong") as response:
            if response.status != 403:
                raise RuntimeError("capability rejected incorrectly")
        with request(url, token) as response:
            result = bounded_json(response)
            passed = result_ok(response.status, result)
        print(json.dumps(result), flush=True)
        # Reusing an invocation cannot rerun DDL or turn a failure into success.
        with request(url, token) as response:
            if response.status != 409:
                raise RuntimeError("one-attempt guard failed")
    except Exception as failure:
        passed = False
        error = str(failure).replace(token, "[redacted]")[:500]
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for log in logs:
            log.close()
        after = {str(item.resolve()): digest(item) for item in inputs}
        unchanged = before == after
        for name in ("probe.mjs", "worker.mjs"):
            copied = root / "worker" / name
            unchanged = unchanged and copied.is_file() and digest(copied) == before[str((source / name).resolve())]
        passed = passed and unchanged
        evidence = {"passed": passed, "result": result, "error": error, "sources_unchanged": unchanged, "sha256_before": before, "sha256_after": after, "scope": "Fixed schema-cookie capability/transaction probe; no FOKS route, activation, latency or HA qualification"}
        (root / "qualification.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
