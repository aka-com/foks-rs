#!/usr/bin/env python3
"""Run native portability tests in a disposable Linux Secret Service session."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    if sys.platform != "linux":
        raise SystemExit("This acceptance harness requires Linux Secret Service.")
    daemon = os.environ.get("FOKS_TEST_KEYRING_DAEMON") or shutil.which("gnome-keyring-daemon")
    if not daemon or not shutil.which("dbus-run-session"):
        raise SystemExit("Install gnome-keyring and dbus-run-session before running native acceptance.")
    daemon = str(Path(daemon).resolve())
    command = sys.argv[1:] or [
        "cargo", "test", "-p", "foks-client-app", "--lib", "portability", "--", "--test-threads=1"
    ]
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="foks-native-tests-") as temporary:
        base = Path(temporary)
        for name in ("data", "runtime", "control"):
            (base / name).mkdir(mode=0o700)
        env = os.environ.copy()
        env.update(
            XDG_DATA_HOME=str(base / "data"),
            XDG_RUNTIME_DIR=str(base / "runtime"),
            FOKS_TEST_NATIVE_PORTABILITY="1",
            FOKS_NATIVE_TEST_ROOT=str(base),
        )
        # The password unlocks only this disposable keyring. HOME is unchanged so
        # Cargo and platform path-lock behavior match ordinary client processes.
        bootstrap = """
import os, subprocess, sys
subprocess.run([sys.argv[1], '--unlock', '--components=secrets',
                '--control-directory', os.environ['FOKS_NATIVE_TEST_ROOT'] + '/control'],
               input=b'isolated-test-password', check=True)
sys.exit(subprocess.run(sys.argv[2:]).returncode)
"""
        return subprocess.run(
            ["dbus-run-session", "--", sys.executable, "-c", bootstrap, daemon, *command],
            cwd=root / "crates" / "foks-client-app", env=env,
        ).returncode


if __name__ == "__main__":
    sys.exit(main())
