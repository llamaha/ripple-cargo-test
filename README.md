# ripple-cargo-test

Reference [ripple](https://github.com/llamaha/ripple) runner.
Subscribes to a patchwave server, waits for `tag.created` events,
clones the tagged state, runs `cargo test --quiet`, and posts the
result back. About 40 lines of real code.

## Install

```bash
cargo install --git https://github.com/llamaha/ripple-cargo-test
```

Or build from source:

```bash
git clone https://github.com/llamaha/ripple-cargo-test
cd ripple-cargo-test
cargo build --release
```

## Configure

Settings live in `config.toml`:

```toml
server = "https://patchwave.example"
token  = "pw_..."

# Optional below.
runner_name     = "ripple-cargo-test"
runner_instance = "host-a-0"
runner_version  = "0.1.0"
runner_role     = "cargo-test"
runner_hostname = "host-a"
runner_repos    = ["owner/repo", "owner/other"]
workspace       = "/var/lib/ripple/work"
```

| Field | Required | Purpose |
|-------|----------|---------|
| `server` | yes | Server base URL, no trailing slash. |
| `token` | yes | API token. Mint via `POST /api/users/{username}/tokens`. |
| `runner_name` | no | Surfaced as the `details.provider` chip on the CI badge AND as the row label in patchwave's Runners dashboard. Defaults to the token sub. |
| `runner_instance` | no | Stable identifier for this process. Two runners with the same `name` on the same host should set distinct instances. |
| `runner_version` | no | Override the SDK's compile-time version shown in the dashboard. |
| `runner_role` | no | Free-form role label shown in the dashboard (e.g. `cargo-test`, `lint`). |
| `runner_hostname` | no | Runner-supplied hostname. The server also captures the source IP independently. |
| `runner_repos` | no | List of `"owner/repo"` strings. Absent = every repo the token can push to. Each entry must already be in the token's access set or the server rejects the connection with 400. |
| `workspace` | no | Scratch dir for checkouts. Defaults to `std::env::temp_dir()`. |

The token's user needs push access to every repo the runner is
expected to react to. The server filters the event stream by push
access; the runner sees nothing for repos it can't push to.

Config-file lookup order:
1. `--config <path>` CLI flag
2. `RIPPLE_CONFIG` env var
3. `./config.toml` in the working dir
4. `/etc/ripple-cargo-test/config.toml`

## Run

```bash
ripple-cargo-test                          # uses ./config.toml or /etc/...
ripple-cargo-test --config /etc/r.toml     # explicit path
RIPPLE_CONFIG=/etc/r.toml ripple-cargo-test
```

It runs forever. Reconnects on transport error with exponential
backoff (500ms → 30s). Drop it under systemd / Nomad / k8s if you
want supervision.

## What it does on each tagged release

1. SSE event arrives: `{kind: "tag.created", owner, repo, payload: {name, state_hash, view}}`.
2. Clone the repo into a scratch dir under `workspace` (or `/tmp`).
3. `cd` into the checkout, run `cargo test --quiet`.
4. POST `/api/ci/{state_hash}/result` with `status: "pass" | "fail"`,
   `details.summary`, `details.duration_ms`, and (if `PATCHWAVE_RUNNER_NAME` is set) `details.provider`.
5. Server flips the tag's CI badge accordingly.

Per-push CI: change `EventKind::TagCreated` to `EventKind::ChangePushed`
in `src/main.rs`. That fires on every `atomic push` instead of every
tag — much noisier, generally not what you want.

## Writing your own runner

Copy `src/main.rs` and swap `cargo test --quiet` for whatever your
project actually needs:

```rust
let ok = checkout.run("./scripts/ci.sh").await?;
```

The full SDK surface (event kinds, report fields, multi-handler
runners) lives in [the ripple repo](https://github.com/llamaha/ripple).

## License

MIT OR Apache-2.0.
