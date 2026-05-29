//! Reference ripple runner.
//!
//! Subscribes to a patchwave server's runner event stream, clones the
//! repo on each `tag.created` event, runs `cargo test --quiet`, and
//! reports `pass` / `fail` back via `/api/ci/{hash}/result`.
//!
//! Why `tag.created` and not `change.pushed`? Patchwave's design says
//! "CI runs at release, not on every change" — tags are the release
//! marker. Switch to `EventKind::ChangePushed` if you want per-push CI.
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
        .on(EventKind::TagCreated, |ctx| async move {
            let checkout = ctx.checkout().await?;
            info!(
                owner = %checkout.owner,
                repo  = %checkout.repo,
                view  = %checkout.view,
                path  = %checkout.path.display(),
                "checked out, running cargo test"
            );

            let started = std::time::Instant::now();
            let ok = checkout.run("cargo test --quiet").await?;
            let duration_ms = started.elapsed().as_millis() as u64;

            ctx.report(if ok { "pass" } else { "fail" })
                .summary(if ok {
                    "cargo test passed"
                } else {
                    "cargo test failed"
                })
                .duration_ms(duration_ms)
                .send()
                .await?;
            Ok(())
        })
        .run()
        .await?;

    Ok(())
}
