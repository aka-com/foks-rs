#!/usr/bin/env python3
"""Exercise onboarding before granting it. Never emit credentials or CLI output."""
import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile

CAPABILITIES = ("signup", "device-administration", "recovery", "passphrases")


class Canary:
    def __init__(self, client, target, root, ca=None, expected_host=None):
        self.client, self.target, self.root, self.ca = client, target, root, ca
        self.expected_host = expected_host
        self.stage = "initialize"

    def argv(self, machine, *args):
        return [self.client, "--state-dir", str(self.root / machine), "--json", *map(str, args)]

    def run(self, machine, *args):
        self.stage = " ".join(map(str, args[:2]))
        result = subprocess.run(self.argv(machine, *args), capture_output=True, timeout=180)
        if result.returncode:
            raise RuntimeError(self.stage)
        return json.loads(result.stdout)

    def machine(self, name):
        self.run(name, "init", "--key-backend", "private-file")
        args = ["profile", "add", "canary", self.target, "--generation", "v019"]
        if self.ca:
            args += ["--ca-der", self.ca]
        self.run(name, *args)
        host = self.run(name, "profile", "probe", "canary")["host_id_hex"]
        if self.expected_host is not None and host != self.expected_host:
            raise RuntimeError("canary host identity mismatch")
        self.expected_host = host

    def rejected(self, machine, *args):
        self.stage = "reject " + " ".join(map(str, args[:2]))
        result = subprocess.run(self.argv(machine, *args), capture_output=True, timeout=180)
        if result.returncode == 0:
            raise RuntimeError(self.stage)

    def secret(self, name, value):
        path = self.root / name
        with path.open("x", encoding="utf8") as output:
            output.write(value)
        return path

    def check(self, email, invite=None):
        self.machine("owner")
        self.machine("peer")
        self.machine("recovered")
        args = ["account", "create", "canary", "owner", "--username",
                "canary" + secrets.token_hex(8), "--device-name", "onboarding canary",
                "--email", email]
        if invite:
            args += ["--invite-file", invite]
        owner = self.run("owner", *args)
        first = self.secret("first-passphrase", secrets.token_urlsafe(32))
        second = self.secret("second-passphrase", secrets.token_urlsafe(32))
        for command, phrase in (("set", first), ("change", second)):
            report = self.run("owner", "passphrase", command, "canary", "owner",
                             "--passphrase-file", phrase, "--passphrase-confirmation-file", phrase)
            if report.get("verified") is not True:
                raise RuntimeError("passphrase verification")
        self.rejected("owner", "passphrase", "verify", "canary", "owner", "--passphrase-file", first)

        offer = self.run("owner", "device", "pair-offer", "canary", "owner")
        phrase = self.secret("pairing-phrase", offer["phrase"])
        # Different state roots are essential: the finisher holds its profile lock.
        with tempfile.TemporaryFile() as finish_output:
            finish = subprocess.Popen(self.argv("owner", "device", "pair-finish", "canary", "owner"),
                                      stdout=finish_output, stderr=subprocess.DEVNULL)
            try:
                peer = self.run("peer", "device", "pair-accept", "canary", "peer",
                                "--phrase-file", phrase, "--device-name", "canary paired device")
                if finish.wait(timeout=180):
                    raise RuntimeError("pair finish")
                finish_output.seek(0)
                if json.load(finish_output)["device_id_hex"] != peer["device_id_hex"]:
                    raise RuntimeError("pair identity mismatch")
            finally:
                if finish.poll() is None:
                    finish.kill()
                    finish.wait()
                phrase.unlink()
        if self.run("peer", "account", "sync", "canary", "peer")["username"] != owner["username"]:
            raise RuntimeError("paired account identity")
        backup_phrase = self.root / "backup-phrase"
        backup = self.run("owner", "recovery", "enroll", "canary", "owner", "backup",
                          "--output", backup_phrase)
        recovered = self.run("recovered", "recovery", "recover", "canary", "recovered",
                             "--phrase-file", backup_phrase, "--device-name", "canary recovered device")
        if self.run("recovered", "account", "sync", "canary", "recovered")["username"] != owner["username"]:
            raise RuntimeError("recovered account identity")
        self.run("owner", "recovery", "revoke", "canary", "owner", "backup", backup["backup_id_hex"])
        for device in (peer, recovered):
            self.run("owner", "device", "revoke", "canary", "owner", device["device_id_hex"])
        devices = self.run("owner", "device", "list", "canary", "owner")
        removed = {peer["device_id_hex"], recovered["device_id_hex"], backup["backup_id_hex"]}
        if any(device["id_hex"] in removed for device in devices):
            raise RuntimeError("revoked key remains active")
        self.rejected("peer", "account", "sync", "canary", "peer")
        self.rejected("recovered", "account", "sync", "canary", "recovered")
        report = self.run("owner", "passphrase", "verify", "canary", "owner", "--passphrase-file", second)
        if report.get("verified") is not True:
            raise RuntimeError("passphrase verification after revocation")
        self.run("owner", "account", "sync", "canary", "owner")
        return CAPABILITIES


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--ca-der")
    parser.add_argument("--expected-host")
    args = parser.parse_args()
    if os.environ.get("FOKS_CANARY_ALLOW_DISPOSABLE_SIGNUP") != "1":
        parser.exit(1, "onboarding canary requires explicit disposable-signup authorization\n")
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="foks-onboarding-canary-") as directory:
        canary = Canary(args.client, args.target, Path(directory), args.ca_der, args.expected_host)
        try:
            capabilities = canary.check(os.environ.get("FOKS_CANARY_SIGNUP_EMAIL", ""),
                                        os.environ.get("FOKS_CANARY_INVITE_FILE") or None)
        except Exception:
            # Neither exception values nor child stdout/stderr are safe to log.
            parser.exit(1, f"onboarding canary failed at {canary.stage}\n")
        print("\n".join(capabilities))


if __name__ == "__main__":
    main()
