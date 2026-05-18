#!/usr/bin/env bash
# Phase 3 atomic deletion: remove the C# authoring layer + every
# `(Author)` prefab now that CSA + SMA outputs are emitted by this rlib.
#
# Run AFTER:
#   1. `scripts/regen-corpus.sh` has been executed and Unity has emitted
#      the new sprite `.asset` corpus through `pipeline::generate`.
#   2. Every SMA prefab has had its `.tps.mesh.json` authored (via
#      `crates/core/examples/sma_dumper.cs` + the companion converter) and Unity
#      has emitted the new Mesh `.asset` corpus through the same
#      pipeline.
#   3. The resulting meow-tower diff has been reviewed + committed.
#
# This script ONLY deletes files. It does not touch git; the user runs
# `git status` / `git diff` / `git add` afterwards to commit.
#
# What gets deleted (under `$MEOW_CLIENT/Assets` + meow-tower packages):
#
#   - 21 C# files under `Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring/`
#     (the entire SMA/CSA authoring tree)
#   - 58 CSA `(Author)` prefabs listed in `/tmp/csa-prefabs-only.txt`
#   - 36 SMA `(Author)` prefabs listed in `/tmp/sma-files.txt`
#     (Box prefabs + the lone non-Box StringLightR)
#
# Each list is regenerated via `unity-assetdb usage <script-guid>`:
#   CSA = 571ad98c7c0d4a559a0cf213d8da355f
#   SMA = d003afe76b0a48aa8f1caad657e5095a
#
# This is destructive. The companion `restore-authoring.sh` doesn't
# exist — use `git checkout` if the user needs to revert.

set -uo pipefail
MEOW_CLIENT="${MEOW_CLIENT:?MEOW_CLIENT env var required}"

cd "$MEOW_CLIENT" || exit 1

# Guard: confirm git is clean enough that the user can revert.
if ! git diff --quiet HEAD; then
    echo "WARNING: meow-tower has uncommitted changes."
    echo "Commit or stash before running this script so the deletion is reviewable."
    exit 1
fi

deleted=0
fail=0
delete() {
    local f="$1"
    if [[ -f "$f" || -d "$f" ]]; then
        rm -rf "$f" && deleted=$((deleted + 1)) || fail=$((fail + 1))
        # `.meta` sidecars must go too.
        if [[ -f "$f.meta" ]]; then
            rm -f "$f.meta" && deleted=$((deleted + 1))
        fi
    fi
}

echo "--- deleting C# authoring tree ---"
sma_dir="Packages/com.boxcat.libs/Core/Runtime/SpriteMeshAuthoring"
for f in "$sma_dir"/CanvasSpriteAuthor.cs \
         "$sma_dir"/ColorTextureUtils.cs \
         "$sma_dir"/GameObjectBuilder.cs \
         "$sma_dir"/MeshData.cs \
         "$sma_dir"/Polygon2DAuthor.cs \
         "$sma_dir"/PolygonRuntimeView.cs \
         "$sma_dir"/PolygonSceneGUIEditor.cs \
         "$sma_dir"/SpriteAtlasAuthoringSettings.cs \
         "$sma_dir"/SpriteAuthoringUtils.cs \
         "$sma_dir"/SpriteBuilder.cs \
         "$sma_dir"/SpriteCombineFeed.cs \
         "$sma_dir"/SpriteMeshAuthor.cs \
         "$sma_dir"/SpriteMeshBuilder.cs \
         "$sma_dir"/SpriteMirrorFeed.cs \
         "$sma_dir"/SpriteMirrorFeed.Editor.cs \
         "$sma_dir"/SpritePatchFeed.cs \
         "$sma_dir"/SpritePatchFeed.Editor.cs \
         "$sma_dir"/SpriteRendererFeed.cs \
         "$sma_dir"/SpriteRendererFeed.Editor.cs \
         "$sma_dir"/Triangulator.cs \
         "$sma_dir"/UIReconstructor.cs ; do
    delete "$f"
done
# After all .cs files are gone, the directory should be empty (apart from
# the auto-generated .meta which `delete` already handled). Remove it.
if [[ -d "$sma_dir" && -z "$(ls -A "$sma_dir" 2>/dev/null)" ]]; then
    rmdir "$sma_dir"
    rm -f "$sma_dir.meta"
fi

echo "--- deleting CSA (Author) prefabs ---"
if [[ -f /tmp/csa-prefabs-only.txt ]]; then
    while IFS= read -r prefab; do
        [[ -z "$prefab" ]] && continue
        delete "$prefab"
    done < /tmp/csa-prefabs-only.txt
else
    echo "WARN: /tmp/csa-prefabs-only.txt missing — regenerate via:"
    echo "  unity-assetdb usage 571ad98c7c0d4a559a0cf213d8da355f \\"
    echo "    | grep '\\.prefab' | sort -u > /tmp/csa-prefabs-only.txt"
fi

echo "--- deleting SMA prefabs ---"
if [[ -f /tmp/sma-files.txt ]]; then
    while IFS= read -r prefab; do
        [[ -z "$prefab" ]] && continue
        case "$prefab" in
            *.prefab) delete "$prefab" ;;
            *) ;; # the list also contains the .cs.meta header; skip
        esac
    done < /tmp/sma-files.txt
else
    echo "WARN: /tmp/sma-files.txt missing — regenerate via:"
    echo "  unity-assetdb usage d003afe76b0a48aa8f1caad657e5095a \\"
    echo "    | grep '\\.prefab' | sort -u > /tmp/sma-files.txt"
fi

echo
echo "--- summary ---"
echo "deleted: $deleted file(s)"
echo "failed:  $fail file(s)"
echo
echo "Review with: git status; git diff --stat HEAD"
echo "Revert with: git restore --staged --worktree :/"
if [[ $fail -gt 0 ]]; then exit 1; fi
