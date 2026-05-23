#!/usr/bin/env bash
# Migrate _prefix from .tps.meta (TPSImporter) to .tpsheet.meta (TPSheetImporter).
#
# For each .tps.meta with a non-empty _prefix, writes a .tpsheet.meta with
# the TPSheetImporter ScriptedImporter block carrying the same _prefix value.
# Skips atlases where the .tpsheet.meta already exists (idempotent).
#
# Prerequisites:
#   - .tpsheet files must exist on disk (run TexturePacker for all atlases first).
#   - Unity must NOT be running (it would reimport and overwrite our metas).
#
# Usage:
#   cd $MEOW_CLIENT
#   scripts/migrate-prefix-to-tpsheet.sh

set -euo pipefail
MEOW_CLIENT="${MEOW_CLIENT:?MEOW_CLIENT env var required}"
cd "$MEOW_CLIENT"

# TPSheetImporter GUID — must match the .meta for TPSheetImporter.cs.
# Read it from the actual file.
IMPORTER_GUID=$(grep "^guid:" Packages/com.boxcat.libs/TexturePacker/TPSheetImporter.cs.meta | awk '{print $2}')
if [[ -z "$IMPORTER_GUID" ]]; then
    echo "error: could not read TPSheetImporter.cs.meta GUID" >&2
    exit 1
fi

migrated=0
skipped=0

while IFS= read -r tps_meta; do
    # Extract _prefix value from .tps.meta
    prefix=$(grep '  _prefix:' "$tps_meta" 2>/dev/null | sed 's/.*_prefix: *//' | head -1)
    if [[ -z "$prefix" ]]; then
        continue
    fi

    # Derive .tpsheet.meta path
    tps_path="${tps_meta%.meta}"
    stem=$(basename "$tps_path" .tps)
    dir=$(dirname "$tps_path")
    tpsheet_meta="$dir/$stem.tpsheet.meta"

    # Skip if .tpsheet doesn't exist (TP hasn't packed this atlas yet)
    if [[ ! -f "$dir/$stem.tpsheet" ]]; then
        continue
    fi

    # Skip if .tpsheet.meta already exists
    if [[ -f "$tpsheet_meta" ]]; then
        skipped=$((skipped + 1))
        continue
    fi

    # Generate a GUID for the .tpsheet.meta (deterministic from path)
    guid=$(echo -n "$tpsheet_meta" | md5 -q | head -c 32)

    cat > "$tpsheet_meta" <<META
fileFormatVersion: 2
guid: $guid
ScriptedImporter:
  internalIDToNameTable: []
  externalObjects: {}
  serializedVersion: 2
  userData:
  assetBundleName:
  assetBundleVariant:
  script: {fileID: 11500000, guid: $IMPORTER_GUID, type: 3}
  _prefix: $prefix
META

    migrated=$((migrated + 1))
    echo "M  $tpsheet_meta  (prefix=$prefix)"
done < <(find Assets -name "*.tps.meta" -type f | sort)

echo
echo "--- summary ---"
echo "migrated: $migrated"
echo "skipped:  $skipped (already exist)"
