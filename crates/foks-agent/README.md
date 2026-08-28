# foks-agent

Bounded resident process for standalone FOKS clients. It listens on a private
Unix-domain socket below an explicit `--state-dir`, runs blocking client work
through a bounded worker semaphore, limits active connections and requests per
connection, and applies read/write/dispatch deadlines.

The resident scheduler polls durable `FoksScheduler` state and runs due user
refreshes only for profiles whose protocol policy permits synchronization.
The agent does not accept secret-bearing signup, mutation, provisioning, or
recovery requests.

```text
foks-agent --state-dir /private/client
```

macOS and Linux are supported. Windows is currently refused because an
authenticated named-pipe implementation and ACL validation have not landed.
Use `foks-rs` directly on Windows until that boundary exists.
