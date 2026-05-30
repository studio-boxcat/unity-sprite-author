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

/// Canonical TexturePacker CLI install path for the current platform. The
/// single source of truth for every consumer (CLI, GUI editor, bridge watch);
/// there's no override (no env, no flag) — every caller packs with this exact
/// binary. `cfg` is evaluated against the *target*, so the bridge's Windows
/// cross-compile picks the `.exe` path without any host branching. GUI hosts
/// (the Unity Editor loading the bridge dylib, the .app editor) don't inherit a
/// shell `PATH` containing TexturePacker's dir, which is why this is the
/// absolute install path rather than a bare-name lookup.
#[cfg(target_os = "macos")]
pub const TEXTUREPACKER_CMD: &str = "/Applications/TexturePacker.app/Contents/MacOS/TexturePacker";
#[cfg(target_os = "windows")]
pub const TEXTUREPACKER_CMD: &str = r"C:\Program Files\CodeAndWeb\TexturePacker\bin\TexturePacker.exe";
// Unknown platform: fall back to a bare-name PATH lookup.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const TEXTUREPACKER_CMD: &str = "texturepacker";

/// Run TexturePackerCLI ([`TEXTUREPACKER_CMD`]) on `tps` from the `.tps`'s
/// parent dir — texturepacker resolves `fileLists` paths relative to its CWD.
/// Errors if the process can't be spawned or exits nonzero.
pub fn pack(tps: &Path) -> Result<(), String> {
    let dir = tps.parent().expect("tps has parent (canonicalized)");
    let name = tps.file_name().expect("tps has filename");
    let status = Command::new(TEXTUREPACKER_CMD)
        .arg(name)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("spawn `{TEXTUREPACKER_CMD} {}`: {e}", name.to_string_lossy()))?;
    if !status.success() {
        return Err(format!(
            "`{TEXTUREPACKER_CMD} {}` exited with {status}",
            name.to_string_lossy()
        ));
    }
    Ok(())
}
