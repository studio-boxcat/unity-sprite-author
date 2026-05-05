#!/usr/bin/env bash
# Move per-atlas `_prefix` from .tpsheet.meta (TPSheetImporter) to
# .tps.meta (TPSImporter). The new emit pipeline deletes the .tpsheet on
# success, so .tpsheet.meta becomes ephemeral; the .tps file is the
# stable home for `_prefix`.
#
# Idempotent: skips .tps.meta files that already use TPSImporter.
# Preserves the .tps.meta `guid:` (don't regenerate — Unity caches the
# DefaultImporter assignment by guid in Library/, regenerating would
# force a global reimport).
#
# Usage:
#   scripts/migrate-tpsheet-meta.sh --dry-run
#   scripts/migrate-tpsheet-meta.sh                       # default = $MEOW_CLIENT
#   scripts/migrate-tpsheet-meta.sh /path/to/meow-tower
#
# Bail conditions:
#   - TPSImporter.cs.meta not found (the script needs the importer's GUID
#     to wire .tps.meta to it).

set -euo pipefail

DRY_RUN=0
ROOT=""
for arg in "$@"; do
    case "$arg" in
        --dry-run|-n) DRY_RUN=1 ;;
        --help|-h)
            sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        -*) echo "unknown flag: $arg" >&2; exit 1 ;;
        *) ROOT="$arg" ;;
    esac
done

ROOT="${ROOT:-${MEOW_CLIENT:-}}"
if [[ -z "$ROOT" ]]; then
    echo "error: meow-tower root not set. Pass as argument or via \$MEOW_CLIENT." >&2
    exit 1
fi
if [[ ! -d "$ROOT/Assets" ]]; then
    echo "error: $ROOT/Assets not found (not a Unity project root?)" >&2
    exit 1
fi

# Discover TPSImporter's script GUID (where .tps.meta will point).
TPS_IMPORTER_META="$ROOT/Assets/50_Modules/Tools/TexturePacker/TPSImporter.cs.meta"
if [[ ! -f "$TPS_IMPORTER_META" ]]; then
    echo "error: $TPS_IMPORTER_META not found." >&2
    echo "       Author TPSImporter.cs first; the script needs its GUID." >&2
    exit 1
fi
SCRIPT_GUID=$(awk '/^guid:/{print $2; exit}' "$TPS_IMPORTER_META")
if [[ -z "$SCRIPT_GUID" ]]; then
    echo "error: could not parse guid from $TPS_IMPORTER_META" >&2
    exit 1
fi
echo "TPSImporter script guid: $SCRIPT_GUID"
echo "Root: $ROOT"
[[ $DRY_RUN -eq 1 ]] && echo "(dry-run)"

migrated=0
skipped_already=0
skipped_no_prefix=0
skipped_no_tps_meta=0

while IFS= read -r -d '' tpsheet_meta; do
    base="${tpsheet_meta%.tpsheet.meta}"
    tps_meta="$base.tps.meta"

    if [[ ! -f "$tps_meta" ]]; then
        skipped_no_tps_meta=$((skipped_no_tps_meta+1))
        continue
    fi

    # Already migrated to OUR TPSImporter? Skip.
    # (Some .tps.meta files in the corpus reference a phantom ScriptedImporter
    # whose .cs.meta is missing — those still need rewriting.)
    if grep -q "script: {fileID: 11500000, guid: $SCRIPT_GUID," "$tps_meta"; then
        skipped_already=$((skipped_already+1))
        continue
    fi

    # Pull _prefix from .tpsheet.meta. Empty values are fine — atlases
    # without a prefix still benefit from the format change because the
    # new pipeline reads from .tps.meta exclusively.
    prefix=$(awk '/^  _prefix:/{ sub(/^  _prefix:[[:space:]]*/, ""); print; exit }' "$tpsheet_meta")
    prefix="${prefix:-}"

    if [[ -z "$prefix" ]]; then
        # Nothing to migrate. Leave .tps.meta as DefaultImporter.
        skipped_no_prefix=$((skipped_no_prefix+1))
        continue
    fi

    tps_guid=$(awk '/^guid:/{print $2; exit}' "$tps_meta")
    if [[ -z "$tps_guid" ]]; then
        echo "  WARN: no guid in $tps_meta — skipping" >&2
        continue
    fi

    new_meta=$(cat <<EOF
fileFormatVersion: 2
guid: $tps_guid
ScriptedImporter:
  internalIDToNameTable: []
  externalObjects: {}
  serializedVersion: 2
  userData:
  assetBundleName:
  assetBundleVariant:
  script: {fileID: 11500000, guid: $SCRIPT_GUID, type: 3}
  _prefix: $prefix
EOF
)

    rel="${tps_meta#$ROOT/}"
    echo "  $rel  prefix=$prefix"
    if [[ $DRY_RUN -eq 0 ]]; then
        printf '%s\n' "$new_meta" > "$tps_meta"
    fi
    migrated=$((migrated+1))
done < <(find "$ROOT/Assets" -type f -name '*.tpsheet.meta' -print0)

echo
echo "Summary:"
echo "  migrated:                $migrated"
echo "  skipped (already done):  $skipped_already"
echo "  skipped (empty prefix):  $skipped_no_prefix"
echo "  skipped (no .tps.meta):  $skipped_no_tps_meta"
[[ $DRY_RUN -eq 1 ]] && echo "  (dry-run; no files written)"
