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
