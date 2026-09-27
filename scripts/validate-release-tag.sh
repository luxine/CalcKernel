#!/usr/bin/env bash
set -euo pipefail

tag="${1:-}"
if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "release tag must use stable vMAJOR.MINOR.PATCH form: ${tag}" >&2
  exit 1
fi

tag_ref="refs/tags/${tag}"
if ! git show-ref --verify --quiet "${tag_ref}"; then
  echo "release tag ref is missing: ${tag_ref}" >&2
  exit 1
fi

object_type="$(git cat-file -t "${tag_ref}")"
if [[ "${object_type}" != "tag" ]]; then
  echo "release tag must be annotated: ${tag}" >&2
  exit 1
fi
