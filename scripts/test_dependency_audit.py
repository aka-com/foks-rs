"""Check the dependency audit configuration and the release build wrappers.

Run with: python3 -m unittest discover -s scripts -p 'test_dependency_audit.py'
"""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

REPOSITORY = Path(__file__).resolve().parent.parent
FOKS_CARGO = REPOSITORY / "scripts" / "foks-cargo.sh"
VERIFY_AUDITABLE = REPOSITORY / "scripts" / "verify-auditable-binaries.sh"
DEPENDABOT = REPOSITORY / ".github" / "dependabot.yml"
DEPENDENCY_WORKFLOW = REPOSITORY / ".github" / "workflows" / "foks-dependencies.yml"


def tracked_files(name):
    """Return repository-relative paths of tracked files with this basename."""
    output = subprocess.run(
        ["git", "ls-files", "--", name, f"**/{name}"],
        cwd=REPOSITORY,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return sorted(Path(line) for line in output.splitlines())


def dependabot_directory(path):
    parent = path.parent.as_posix()
    return "/" if parent == "." else f"/{parent}"


def dependabot_directories(text):
    """Map each package ecosystem in dependabot.yml to its directories."""
    ecosystems = {}
    ecosystem = None
    in_list = False
    for line in text.splitlines():
        stripped = line.strip()
        match = re.fullmatch(r"- package-ecosystem:\s*(\S+)", stripped)
        if match:
            ecosystem = match.group(1)
            ecosystems.setdefault(ecosystem, set())
            in_list = False
            continue
        if ecosystem is None:
            continue
        match = re.fullmatch(r"directory:\s*[\"']?([^\"'\s]+)[\"']?", stripped)
        if match:
            ecosystems[ecosystem].add(match.group(1))
            continue
        if stripped == "directories:":
            in_list = True
            continue
        if in_list:
            if stripped.startswith("#"):
                continue
            match = re.fullmatch(r"-\s*[\"']?([^\"'\s]+)[\"']?", stripped)
            if match:
                ecosystems[ecosystem].add(match.group(1))
                continue
            in_list = False
    return ecosystems


def exact_pins():
    """Yield (manifest, line number, crate, line, previous line) for each
    dependency in a tracked Cargo.toml that requires an exact version."""
    exact = re.compile(r"""^([\w-]+)\s*=\s*(?:\{.*\bversion\s*=\s*)?["']=""")
    for manifest in tracked_files("Cargo.toml"):
        lines = (REPOSITORY / manifest).read_text().splitlines()
        for index, line in enumerate(lines):
            match = exact.match(line.strip())
            if match:
                previous = lines[index - 1].strip() if index > 0 else ""
                yield manifest, index + 1, match.group(1), line, previous


def write_executable(path, body):
    path.write_text(body)
    path.chmod(0o755)


class DependencyCoverageTests(unittest.TestCase):
    def test_dependabot_lists_every_manifest_directory(self):
        configured = dependabot_directories(DEPENDABOT.read_text())
        expected = {
            "cargo": {dependabot_directory(path) for path in tracked_files("Cargo.lock")},
            "npm": {dependabot_directory(path) for path in tracked_files("package-lock.json")},
            "gomod": {dependabot_directory(path) for path in tracked_files("go.mod")},
            "github-actions": {"/"},
        }
        for ecosystem, directories in expected.items():
            with self.subTest(ecosystem=ecosystem):
                self.assertTrue(directories, f"no {ecosystem} manifests found")
                self.assertEqual(configured.get(ecosystem), directories)

    def test_dependabot_keeps_the_go_foks_oracle_at_v0_1_9(self):
        text = DEPENDABOT.read_text()
        for module in [
            "github.com/foks-proj/go-foks",
            "github.com/foks-proj/go-snowpack-rpc",
            "github.com/keybase/go-codec",
        ]:
            with self.subTest(module=module):
                self.assertIn(f'- dependency-name: "{module}"', text)

    def test_cargo_deny_checks_every_lockfile(self):
        match = re.search(r"^\s*manifest:\s*\[([^\]]*)\]", DEPENDENCY_WORKFLOW.read_text(), re.M)
        self.assertIsNotNone(match, "cargo-deny manifest matrix not found")
        manifests = {item.strip() for item in match.group(1).split(",")}
        expected = {(path.parent / "Cargo.toml").as_posix() for path in tracked_files("Cargo.lock")}
        self.assertEqual(manifests, expected)

    def test_exact_version_requirements_state_a_reason(self):
        undocumented = [
            f"{manifest}:{number}: {line.strip()}"
            for manifest, number, _, line, previous in exact_pins()
            if "#" not in line and not previous.startswith("#")
        ]
        self.assertEqual(undocumented, [], "exact pins need a comment saying why")

    def test_exact_pins_are_excluded_from_grouped_cargo_updates(self):
        match = re.search(r"^\s*exclude-patterns:\s*\[([^\]]*)\]", DEPENDABOT.read_text(), re.M)
        self.assertIsNotNone(match, "cargo-patch exclude-patterns not found")
        excluded = {item.strip().strip("\"'") for item in match.group(1).split(",")}
        pinned = {name for _, _, name, _, _ in exact_pins()}
        self.assertTrue(pinned, "no exact pins found")
        self.assertEqual(pinned - excluded, set())


class FoksCargoTests(unittest.TestCase):
    def run_wrapper(self, auditable_installed, value):
        with tempfile.TemporaryDirectory() as directory:
            bin_directory = Path(directory)
            log = bin_directory / "cargo.log"
            write_executable(bin_directory / "cargo", f'#!/bin/sh\necho "$@" > "{log}"\n')
            if auditable_installed:
                write_executable(bin_directory / "cargo-auditable", "#!/bin/sh\nexit 0\n")
            environment = {"PATH": f"{bin_directory}:/usr/bin:/bin"}
            if value is not None:
                environment["FOKS_AUDITABLE_BUILD"] = value
            result = subprocess.run(
                [str(FOKS_CARGO), "build", "--locked", "--release"],
                env=environment,
                capture_output=True,
                text=True,
            )
            invoked = log.read_text().strip() if log.exists() else None
            return result, invoked

    def test_runs_plain_cargo_when_unset(self):
        result, invoked = self.run_wrapper(auditable_installed=True, value=None)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(invoked, "build --locked --release")

    def test_runs_cargo_auditable_when_requested(self):
        result, invoked = self.run_wrapper(auditable_installed=True, value="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(invoked, "auditable build --locked --release")

    def test_refuses_without_cargo_auditable(self):
        result, invoked = self.run_wrapper(auditable_installed=False, value="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires cargo-auditable", result.stderr)
        self.assertIsNone(invoked)

    def test_refuses_unrecognized_values(self):
        result, invoked = self.run_wrapper(auditable_installed=True, value="true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIsNone(invoked)


class VerifyAuditableBinariesTests(unittest.TestCase):
    def run_verify(self, executables, extra_arguments=()):
        """Run the verifier with a stand-in rust-audit-info that prints the
        content of each "executable" file, or fails when it is empty."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_executable(
                root / "rust-audit-info",
                '#!/bin/sh\nif [ -s "$1" ]; then cat "$1"; else echo "No audit data found" >&2; exit 1; fi\n',
            )
            arguments = []
            for name, content, package in executables:
                (root / name).write_text(content)
                arguments += [str(root / name), package]
            python_directory = Path(sys.executable).parent
            environment = {"PATH": f"{root}:{python_directory}:/usr/bin:/bin"}
            return subprocess.run(
                [str(VERIFY_AUDITABLE), *arguments, *extra_arguments],
                env=environment,
                capture_output=True,
                text=True,
            )

    @staticmethod
    def audit_data(root):
        return (
            '{"packages":[{"name":"%s","version":"0.4.0","source":"local","dependencies":[1],"root":true},'
            '{"name":"serde","version":"1.0.0","source":"crates.io"}],"format":1}' % root
        )

    def test_accepts_embedded_lists_with_the_expected_roots(self):
        result = self.run_verify(
            [
                ("foks-desktop", self.audit_data("foks-desktop-app"), "foks-desktop-app"),
                ("foks-agent", self.audit_data("foks-agent"), "foks-agent"),
            ]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("2 packages embedded, root foks-agent", result.stdout)

    def test_rejects_an_executable_without_embedded_data(self):
        result = self.run_verify(
            [
                ("foks-desktop", self.audit_data("foks-desktop-app"), "foks-desktop-app"),
                ("foks-agent", "", "foks-agent"),
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("foks-agent has no embedded dependency list", result.stderr)

    def test_rejects_a_missing_executable(self):
        result = self.run_verify([], extra_arguments=["/nonexistent/foks-agent", "foks-agent"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/nonexistent/foks-agent does not exist", result.stderr)

    def test_rejects_a_different_root_package(self):
        result = self.run_verify([("foks-agent", self.audit_data("foks-cli"), "foks-agent")])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected ['foks-agent']", result.stderr)

    def test_rejects_unpaired_arguments(self):
        result = self.run_verify([], extra_arguments=["foks-agent"])
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
