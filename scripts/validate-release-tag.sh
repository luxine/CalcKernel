#!/usr/bin/env bash
set -euo pipefail

tag="${1:-}"
tag_ref="${2:-refs/tags/${tag}}"
event_commit="${3:-}"
if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "release tag must use stable vMAJOR.MINOR.PATCH form: ${tag}" >&2
  exit 1
fi
if [[ "${tag_ref}" != "refs/tags/${tag}" && "${tag_ref}" != "refs/release-validation/${tag}" ]]; then
  echo "release tag ref must match the requested tag: ${tag_ref}" >&2
  exit 1
fi

if ! git show-ref --verify --quiet "${tag_ref}"; then
  echo "release tag ref is missing: ${tag_ref}" >&2
  exit 1
fi

object_type="$(git cat-file -t "${tag_ref}")"
if [[ "${object_type}" != "tag" ]]; then
  echo "release tag must be annotated: ${tag}" >&2
  exit 1
fi

tag_commit="$(git rev-parse --verify "${tag_ref}^{}")"
head_commit="$(git rev-parse --verify HEAD)"
if [[ -n "${event_commit}" && "${head_commit}" != "${event_commit}" ]]; then
  echo "release checkout does not match the triggering event commit: ${tag}" >&2
  exit 1
fi
if [[ "${tag_commit}" != "${head_commit}" ]]; then
  echo "release tag does not point at the checked-out event commit: ${tag}" >&2
  exit 1
fi
