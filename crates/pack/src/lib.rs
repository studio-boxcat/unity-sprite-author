//! Pack orchestration: the pre-pack `Color_*.png` synthesis plus the
//! TexturePackerCLI shell-out, factored out of the CLI bin so both the offline
//! one-shot (`unity-sprite-author-cli`) and the boxcat bridge's Editor-hosted
//! watch share one implementation.
//!
//! Generate (`.tpsheet → .asset`) stays in `unity-sprite-author` core; this
//! crate only covers the *upstream* `.tps → .tpsheet` step. Meta minting
//! (`unity-assetdb` `register`) is the CLI's concern (the Editor already has
//! the metas), so it deliberately lives outside this crate.

mod color_png;
mod color_synth;

pub use color_synth::{SynthError, SynthOutcome, synthesize_for_tps};

use std::path::Path;
use std::process::Command;

// Canonical TexturePacker CLI install path per platform. GUI-launched hosts
// (the Unity Editor loading the bridge dylib, the .app editor) don't inherit a
// shell `PATH` containing TexturePacker's dir, so a bare `texturepacker` lookup
// fails there — point at CodeAndWeb's default install location instead. `cfg`
// is evaluated against the *target*, so the bridge's Windows cross-compile
// picks the `.exe` path without any host branching. Override via
// `$TEXTUREPACKER` (see [`texturepacker_cmd`]).
#[cfg(target_os = "macos")]
const TEXTUREPACKER_DEFAULT: &str = "/Applications/TexturePacker.app/Contents/MacOS/TexturePacker";
#[cfg(target_os = "windows")]
const TEXTUREPACKER_DEFAULT: &str =
    r"C:\Program Files\CodeAndWeb\TexturePacker\bin\TexturePacker.exe";
// Unknown platform: fall back to a bare-name PATH lookup.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TEXTUREPACKER_DEFAULT: &str = "texturepacker";

/// Resolve the TexturePacker CLI command: `$TEXTUREPACKER` if set, else the
/// platform's canonical install path. The single source of truth for every
/// consumer (CLI default, GUI editor, bridge watch) — callers with their own
/// override (e.g. the CLI's `--texturepacker` flag) layer it on top.
pub fn texturepacker_cmd() -> String {
    std::env::var("TEXTUREPACKER").unwrap_or_else(|_| TEXTUREPACKER_DEFAULT.to_string())
}

/// Run TexturePackerCLI (`cmd`) on `tps` from the `.tps`'s parent dir —
/// texturepacker resolves `fileLists` paths relative to its CWD. Errors if the
/// process can't be spawned or exits nonzero.
pub fn pack(cmd: &str, tps: &Path) -> Result<(), String> {
    let dir = tps.parent().expect("tps has parent (canonicalized)");
    let name = tps.file_name().expect("tps has filename");
    let status = Command::new(cmd)
        .arg(name)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("spawn `{cmd} {}`: {e}", name.to_string_lossy()))?;
    if !status.success() {
        return Err(format!(
            "`{cmd} {}` exited with {status}",
            name.to_string_lossy()
        ));
    }
    Ok(())
}
