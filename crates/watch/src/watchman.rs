// Watchman wire layer — sync facade over the async watchman_client.
//
// Mirrors the unity-assetdb/src/watch.rs pattern: per-call current-thread
// tokio runtime (~µs cost), one-shot clock queries (not subscriptions).
// See docs/refresh.md in unity-assetdb for the design rationale.

use std::path::{Path, PathBuf};

use watchman_client::Error as WatchmanError;
use watchman_client::prelude::*;

#[derive(Debug)]
pub enum Delta {
    Fresh { new_clock: String },
    Touched { hints: Vec<String>, new_clock: String },
}

#[derive(Debug)]
pub enum WatchError {
    Unavailable,
    Query(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "watchman unavailable"),
            Self::Query(e) => write!(f, "watchman query: {e}"),
        }
    }
}

impl std::error::Error for WatchError {}

pub fn since(
    project_root: &Path,
    prev_clock: Option<&str>,
) -> Result<Delta, WatchError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| WatchError::Query(format!("build tokio runtime: {e}")))?;
    rt.block_on(since_inner(project_root, prev_clock))
}

async fn since_inner(
    project_root: &Path,
    prev_clock: Option<&str>,
) -> Result<Delta, WatchError> {
    let client = new_connector().connect().await.map_err(map_connect_err)?;

    let canonical = CanonicalPath::canonicalize(project_root)
        .map_err(|e| WatchError::Query(format!("canonicalize: {e}")))?;
    let resolved = client
        .resolve_root(canonical)
        .await
        .map_err(|e| WatchError::Query(format!("{e}")))?;

    let expression = Expr::All(vec![
        Expr::DirName(DirNameTerm {
            path: PathBuf::from("Assets"),
            depth: None,
        }),
        Expr::Suffix(vec![PathBuf::from("tpsheet")]),
    ]);

    let request = QueryRequestCommon {
        since: prev_clock.map(|c| Clock::Spec(ClockSpec::StringClock(c.to_owned()))),
        expression: Some(expression),
        ..Default::default()
    };

    let result = client
        .query::<NameOnly>(&resolved, request)
        .await
        .map_err(|e| WatchError::Query(format!("{e}")))?;

    let new_clock = clock_to_string(result.clock)?;

    if result.is_fresh_instance {
        return Ok(Delta::Fresh { new_clock });
    }

    let hints: Vec<String> = result
        .files
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| row.name.into_inner().into_os_string().into_string().ok())
        .collect();

    Ok(Delta::Touched { hints, new_clock })
}

fn clock_to_string(clock: Clock) -> Result<String, WatchError> {
    match clock {
        Clock::Spec(ClockSpec::StringClock(s)) => Ok(s),
        Clock::Spec(ClockSpec::UnixTimestamp(t)) => Ok(t.to_string()),
        Clock::ScmAware(_) => Err(WatchError::Query(
            "watchman returned SCM-aware clock; not requested".into(),
        )),
    }
}

/// macOS GUI apps (launched from Unity Hub / Finder) don't inherit the
/// shell's PATH, so `Connector::new()` can't find the `watchman` binary
/// via bare name lookup. Fall back to the Homebrew install path.
fn new_connector() -> Connector {
    let c = Connector::new();
    if std::env::var_os("WATCHMAN_SOCK").is_some() {
        eprintln!("sprite-author watch: WATCHMAN_SOCK set, skipping CLI discovery");
        return c;
    }
    #[cfg(target_os = "macos")]
    {
        let brew = Path::new("/opt/homebrew/bin/watchman");
        if brew.exists() {
            eprintln!("sprite-author watch: using {}", brew.display());
            return c.watchman_cli_path(brew);
        } else {
            eprintln!("sprite-author watch: {} not found", brew.display());
        }
    }
    c
}

fn map_connect_err(e: WatchmanError) -> WatchError {
    match &e {
        WatchmanError::ConnectionDiscovery { watchman_path, reason, stderr } => {
            eprintln!(
                "sprite-author watch: discovery failed: path={} reason={reason} stderr={stderr}",
                watchman_path.display()
            );
        }
        WatchmanError::Connect { endpoint, .. } => {
            eprintln!("sprite-author watch: connect failed: endpoint={}", endpoint.display());
        }
        _ => {}
    }
    match e {
        WatchmanError::ConnectionDiscovery { .. } | WatchmanError::Connect { .. } => {
            WatchError::Unavailable
        }
        other => WatchError::Query(format!("{other}")),
    }
}
