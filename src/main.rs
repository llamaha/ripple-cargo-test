//! Reference ripple runner.
//!
//! Subscribes to a patchwave server's runner event stream, clones the
//! repo on each `change.pushed` event, runs `cargo test --quiet`, and
//! reports `pass` / `fail` back via `/api/ci/{hash}/result`.
//!
//! Per-push CI is the noisy default. Patchwave's release model is
//! "CI runs at the tag" — once `atomic tag` syncs through the CLI,
//! flip `EventKind::ChangePushed` to `EventKind::TagCreated` to get
//! one CI run per release instead of one per push.
//!
//! ## Run it
//!
//! ```bash
//! export PATCHWAVE_URL=https://your-server.example
//! export PATCHWAVE_TOKEN=...        # mint via POST /api/users/{u}/tokens
//! export PATCHWAVE_RUNNER_NAME=ripple-cargo-test
//! cargo run --release
//! ```

use ripple::event::EventKind;
use ripple::Runner;
use tracing::info;

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
        .run()
        .await?;

    Ok(())
}
