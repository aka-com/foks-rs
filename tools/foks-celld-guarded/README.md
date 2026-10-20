# Guarded celld runtime tooling

`prepare-runtime.py` prepares the pinned celld inputs, patches and
qualification stages, and `run-node.py` runs a bounded local celld node. Both run only on the macOS lab host: they
require Darwin on arm64, and `node.lock.json` pins interpreter and library
paths under `/private/tmp/foks-celld-lab` and `/opt/homebrew` as the
reviewed roster. A change to those paths, such as a Homebrew Python update,
changes the roster and must be reviewed on that host.

The unit tests follow the same boundary. On any other host,
`test_run_node.py` and `test_prepare_runtime.py` each report one skipped
test, and the `test_check_graph.py` case that needs macOS `sandbox-exec` is
skipped; the rest of `test_check_graph.py` runs anywhere:

```console
python3 -m unittest discover -p 'test*.py'
```
