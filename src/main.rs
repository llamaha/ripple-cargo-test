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
//! ## Configure
//!
//! Settings live in a `config.toml`:
//!
//! ```toml
//! server = "https://patchwave.example.com"
//! token  = "pw_..."
//!
//! # All of the following are optional.
//! runner_name     = "ripple-cargo-test"
//! runner_instance = "host-a-0"
//! runner_version  = "0.1.0"
//! runner_role     = "cargo-test"
//! runner_hostname = "host-a"
//! runner_repos    = ["root/cargo-smoke"]
//! workspace       = "/var/lib/ripple/work"
//! ```
//!
//! The runner picks the config file in this order:
//! 1. `--config <path>` CLI flag,
//! 2. `RIPPLE_CONFIG` env var,
//! 3. `./config.toml` in the current dir,
//! 4. `/etc/ripple-cargo-test/config.toml`.

use ripple::config::Config;
use ripple::event::EventKind;
use ripple::Runner;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Wire shape of `config.toml`. Mirrors [`ripple::config::Config`] but
/// every field is plain `Option<String>` / friendly types so a missing
/// key in the file is just `None` rather than a parse error.
#[derive(Debug, Deserialize)]
struct FileConfig {
    server: String,
    token: String,
    runner_name: Option<String>,
    runner_instance: Option<String>,
    runner_version: Option<String>,
    runner_role: Option<String>,
    runner_hostname: Option<String>,
    runner_repos: Option<Vec<String>>,
    workspace: Option<PathBuf>,
}

impl FileConfig {
    fn into_config(self) -> Config {
        Config {
            server: self.server.trim_end_matches('/').to_string(),
            token: self.token,
            runner_name: self.runner_name,
            runner_instance: self.runner_instance,
            runner_version: self
                .runner_version
                .or_else(|| Some(env!("CARGO_PKG_VERSION").to_string())),
            runner_role: self.runner_role,
            runner_hostname: self.runner_hostname,
            runner_repos: self.runner_repos,
            workspace: self.workspace.unwrap_or_else(std::env::temp_dir),
        }
    }
}

/// Resolve the config-file path using the documented precedence.
fn resolve_config_path() -> anyhow::Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("--config requires a path"))?;
            return Ok(PathBuf::from(path));
        }
        if let Some(rest) = arg.strip_prefix("--config=") {
            return Ok(PathBuf::from(rest));
        }
    }
    if let Ok(path) = std::env::var("RIPPLE_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let cwd_path = Path::new("config.toml");
    if cwd_path.exists() {
        return Ok(cwd_path.to_path_buf());
    }
    Ok(PathBuf::from("/etc/ripple-cargo-test/config.toml"))
}

fn load_config() -> anyhow::Result<Config> {
    let path = resolve_config_path()?;
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let file: FileConfig = toml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
    info!(config = %path.display(), "loaded config");
    Ok(file.into_config())
}

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

    let cfg = load_config()?;

    Runner::from_config(cfg)?
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
