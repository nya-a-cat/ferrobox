#!/usr/bin/env bash
set -euo pipefail

shopt -s nullglob
workflow_files=(.github/workflows/*.yml .github/workflows/*.yaml)
if ((${#workflow_files[@]} == 0)); then
    echo "No GitHub workflow files were found." >&2
    exit 2
fi

action_ref_count=0
while IFS= read -r action_ref; do
    case "${action_ref}" in
        ./*) ;;
        *)
            if [[ ! "${action_ref}" =~ ^[^@]+@[0-9a-f]{40}$ ]]; then
                echo "GitHub Action is not pinned to a full commit SHA: ${action_ref}" >&2
                exit 3
            fi
            ;;
    esac
    action_ref_count=$((action_ref_count + 1))
done < <(
    sed -nE \
        's/^[[:space:]]*(-[[:space:]]*)?uses:[[:space:]]+([^[:space:]#]+).*/\2/p' \
        "${workflow_files[@]}" |
        sort -u
)

if ((action_ref_count == 0)); then
    echo "No GitHub Action references were found." >&2
    exit 2
fi

printf 'Verified %d unique GitHub Action commit pins.\n' "${action_ref_count}"
