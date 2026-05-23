// Watchman-based poller for the TexturePacker → sprite-author pipeline.
//
// Replaces Unity's TPSheetPostprocessor: detects .tpsheet writes via
// watchman clock queries and runs pipeline::generate directly. The
// .tpsheet is retained (pipeline no longer deletes it) so TexturePacker's
// smartUpdateKey hash check works on next GUI publish.
//
// The bridge cdylib wraps this as bxc_sprite_author_watch_{init,poll};
// C# calls poll() from EditorApplication.update every ~2s.
//
// > **Related:** [[TODO.md#watchman-watcher-remaining]], [[CLAUDE.md]]

mod watchman;

use std::path::{Path, PathBuf};

use unity_sprite_author::meta;
use unity_sprite_author::pipeline::{self, GenerateInputs, StandardLayout};

#[derive(Debug)]
pub enum PollResult {
    Idle,
    Processed(usize),
}

pub struct Watcher {
    project_root: PathBuf,
    clock: Option<String>,
}

impl Watcher {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root, clock: None }
    }

    pub fn poll(&mut self) -> PollResult {
        let delta = match watchman::since(&self.project_root, self.clock.as_deref()) {
            Ok(d) => d,
            Err(watchman::WatchError::Unavailable) => return PollResult::Idle,
            Err(e) => {
                eprintln!("sprite-author watch: {e}");
                return PollResult::Idle;
            }
        };

        let (hints, new_clock) = match delta {
            watchman::Delta::Fresh { new_clock } => {
                self.clock = Some(new_clock);
                return PollResult::Idle;
            }
            watchman::Delta::Touched { hints, new_clock } => (hints, new_clock),
        };

        self.clock = Some(new_clock);

        if hints.is_empty() {
            return PollResult::Idle;
        }

        let mut processed = 0usize;
        for hint in &hints {
            if !hint.ends_with(".tpsheet") {
                continue;
            }
            let tpsheet_path = self.project_root.join(hint);
            if !tpsheet_path.exists() {
                continue;
            }
            match self.process_tpsheet(&tpsheet_path) {
                Ok(()) => processed += 1,
                Err(e) => eprintln!("sprite-author watch: {}: {e}", tpsheet_path.display()),
            }
        }

        if processed > 0 { PollResult::Processed(processed) } else { PollResult::Idle }
    }

    fn process_tpsheet(&self, tpsheet_path: &Path) -> Result<(), String> {
        let layout = StandardLayout::from_tpsheet(tpsheet_path)
            .map_err(|e| format!("{e}"))?;

        let prefix = meta::read_tps_prefix(&layout.tps_path).unwrap_or_default();
        let ppu = meta::read_png_ppu(&layout.atlas_png_path).ok_or_else(|| {
            format!("{}.meta missing spritePixelsToUnits", layout.atlas_png_path.display())
        })?;

        let inputs = GenerateInputs {
            tpsheet_path,
            tps_path: &layout.tps_path,
            atlas_png_path: &layout.atlas_png_path,
            sprite_dir: &layout.sprite_dir,
            prefix: &prefix,
            ppu,
        };

        pipeline::generate(&inputs).map_err(|e| format!("{e:#}"))?;
        Ok(())
    }
}
