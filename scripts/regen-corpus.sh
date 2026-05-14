#!/usr/bin/env bash
# Phase 1b corpus regen: walk every .tps under $MEOW_CLIENT/Assets and run
# TexturePackerCLI on it. Outputs an updated .tpsheet (+ deterministic .png)
# next to each .tps. Unity must NOT be running — the postprocessor would
# fire mid-loop and double-process.
#
# Workflow (run after BoxcatBridge has been bumped to a unity-sprite-author
# rev ≥ e23f2b3):
#   1. Quit Unity (`unity-launcher quit` from $MEOW_CLIENT).
#   2. Run this script.
#   3. Reopen Unity. TPSheetPostprocessor fires on each `.tpsheet`,
#      BoxcatBridge dispatches into `pipeline::generate`, sprite `.asset`
#      files are emitted from the fresh tpsheet data.
#   4. `git diff` in meow-tower shows the migration's net effect — the
#      committed `.asset` corpus is replaced wholesale with bytes that
#      reflect current TexturePacker output. Review + commit.
#
# Safe to re-run: TexturePackerCLI is deterministic given the source PNGs.
# Idempotent except for `.png` mtime (content stays byte-stable).
#
# Outputs a summary at the end: atlases processed, failures (if any).

set -uo pipefail
MEOW_CLIENT="${MEOW_CLIENT:?MEOW_CLIENT env var required}"

cd "$MEOW_CLIENT" || exit 1

ok=0
fail=0
fail_list=()

# Skip non-Assets paths and Library/Temp.
tps_files=()
while IFS= read -r line; do
    tps_files+=("$line")
done < <(find Assets -name "*.tps" -type f -print | sort)

echo "Found ${#tps_files[@]} .tps files. Processing…"
for tps in "${tps_files[@]}"; do
    dir=$(dirname "$tps")
    name=$(basename "$tps")
    if (cd "$dir" && texturepacker "$name" >/dev/null 2>&1); then
        ok=$((ok + 1))
    else
        fail=$((fail + 1))
        fail_list+=("$tps")
    fi
done

echo
echo "--- summary ---"
echo "ok:     $ok"
echo "failed: $fail"
if [[ $fail -gt 0 ]]; then
    echo
    echo "failures:"
    printf '  %s\n' "${fail_list[@]}"
    exit 1
fi
