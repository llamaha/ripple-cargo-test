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

| Env var | Required | Purpose |
|---------|----------|---------|
| `PATCHWAVE_URL` | yes | Server base URL, no trailing slash. e.g. `https://patchwave.example` |
| `PATCHWAVE_TOKEN` | yes | API token. Mint via `POST /api/users/{username}/tokens`. |
| `PATCHWAVE_RUNNER_NAME` | no | Surfaced as the `details.provider` chip on the CI badge. |
| `PATCHWAVE_RUNNER_WORKSPACE` | no | Scratch dir for checkouts. Defaults to `std::env::temp_dir()`. |

The token's user needs push access to every repo the runner is
expected to react to. The server filters the event stream by push
access; the runner sees nothing for repos it can't push to.

## Run

```bash
export PATCHWAVE_URL=https://patchwave.example
export PATCHWAVE_TOKEN=pw_...
export PATCHWAVE_RUNNER_NAME=ripple-cargo-test
ripple-cargo-test
```

It runs forever. Reconnects on transport error with exponential
backoff (500ms → 30s). Drop it under systemd / Nomad / k8s if you
want supervision.

## What it does on each tagged release

1. SSE event arrives: `{kind: "tag.created", owner, repo, payload: {name, state_hash, view}}`.
2. Clone the repo into a scratch dir under `PATCHWAVE_RUNNER_WORKSPACE` (or `/tmp`).
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
