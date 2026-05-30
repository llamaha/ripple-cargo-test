//! Reference ripple runner.
//!
//! Two independent handlers wired against one SSE stream:
//!
//! - **`change.pushed`** — clones the repo on every `atomic push`, runs
//!   `cargo test`, reports per-change. Noisy by design; useful while
//!   iterating locally.
//!
//! - **`ci_stage`** — when a tag is created on a repo whose CI stages
//!   use the `sse://` scheme, the server broadcasts one event per
//!   stage. Stage name → command lookup (`docs` → `cargo doc`, `test` →
//!   `cargo test`, `deploy` → `echo Deployed`) with a wildcard fallback
//!   for unknown stages. Each result reports back with `details.stage`
//!   set automatically by `RunnerContext::report`, so the server
//!   advances the pipeline to the next stage (or marks the tag
//!   passed/failed) without any further runner cooperation.
//!
//! ## Run it
//!
//! ```bash
//! export PATCHWAVE_URL=https://your-server.example
//! export PATCHWAVE_TOKEN=...        # mint via POST /api/users/{u}/tokens
//! export PATCHWAVE_RUNNER_NAME=ripple-cargo-test
//! cargo run --release
//! ```
//!
//! ## Smoke test the stage pipeline
//!
//! With the runner up:
//!
//! ```bash
//! # 1. Configure three stages on the target repo (any sse:// URL — the
//! #    scheme is the only thing that matters).
//! for s in docs test deploy; do
//!   curl -X POST "$PATCHWAVE_URL/api/repos/$OWNER/$REPO/ci-stages" \
//!     -H "Authorization: Bearer $PATCHWAVE_TOKEN" \
//!     -H 'Content-Type: application/json' \
//!     -d "{\"name\":\"$s\",\"webhook_url\":\"sse://ripple\"}"
//! done
//!
//! # 2. Create a tag — this fires stage 1 (`docs`). The runner picks
//! #    it off the SSE stream, reports back, and the server walks the
//! #    pipeline forward.
//! curl -X POST "$PATCHWAVE_URL/api/repos/$OWNER/$REPO/tags" \
//!   -H "Authorization: Bearer $PATCHWAVE_TOKEN" \
//!   -H 'Content-Type: application/json' \
//!   -d '{"name":"v0.1.0","view":"dev","run_ci":true}'
//!
//! # 3. Watch the Tags tab — three per-stage badges flip in order.
//! ```

use ripple::event::EventKind;
use ripple::Runner;
use tracing::{info, warn};

/// Map a stage name to the shell command the runner will execute.
/// Unknown stage names fall back to a no-op `echo` so the pipeline
/// keeps moving instead of stalling on a typo.
fn command_for_stage(stage: &str) -> String {
    match stage {
        "docs"   => "cargo doc --no-deps --quiet".into(),
        "test"   => "cargo test --quiet".into(),
        "deploy" => "echo 'deploy: (demo no-op)'".into(),
        other => {
            warn!(stage = %other, "unknown stage; running no-op echo");
            format!("echo 'no command configured for stage {other}'")
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ripple=debug".into()),
        )
        .init();

    info!("ripple-cargo-test starting");

    Runner::from_env()?
        .on(EventKind::ChangePushed, |ctx| async move {
            let checkout = ctx.checkout().await?;
            info!(
                owner = %checkout.owner,
                repo  = %checkout.repo,
                view  = %checkout.view,
                path  = %checkout.path.display(),
                "checked out, running cargo test"
            );

            let started = std::time::Instant::now();
            let (ok, output) = checkout.run_capture("cargo test").await?;
            let duration_ms = started.elapsed().as_millis() as u64;

            // attach_log tiers automatically: small logs inline,
            // larger ones gzip + blob-upload + log_blob reference.
            ctx.report(if ok { "pass" } else { "fail" })
                .summary(if ok {
                    "cargo test passed"
                } else {
                    "cargo test failed"
                })
                .duration_ms(duration_ms)
                .attach_log(&output)
                .await?
                .send()
                .await?;
            Ok(())
        })
        .on(EventKind::CiStage, |ctx| async move {
            // `event.stage()` is `Some(name)` for `CiStage`; the
            // checkout uses `event.change_hash()` (== the tag's state
            // hash) so every stage in a pipeline checks out the same
            // revision.
            let stage = ctx.event.stage().unwrap_or("?").to_string();
            let checkout = ctx.checkout().await?;
            let cmd = command_for_stage(&stage);

            info!(
                owner = %checkout.owner,
                repo  = %checkout.repo,
                stage = %stage,
                cmd   = %cmd,
                path  = %checkout.path.display(),
                "ci_stage: running"
            );

            let started = std::time::Instant::now();
            let (ok, output) = checkout.run_capture(&cmd).await?;
            let duration_ms = started.elapsed().as_millis() as u64;

            // `report()` auto-fills `details.stage = stage` from the
            // event, so the server's `advance_stage` helper can flip
            // the right stage row without any extra plumbing here.
            ctx.report(if ok { "pass" } else { "fail" })
                .summary(format!(
                    "{stage}: {}",
                    if ok { "passed" } else { "failed" }
                ))
                .duration_ms(duration_ms)
                .attach_log(&output)
                .await?
                .send()
                .await?;
            Ok(())
        })
        .run()
        .await?;

    Ok(())
}
