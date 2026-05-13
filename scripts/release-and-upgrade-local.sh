#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OWNER="${OWNER:-life2you}"
TAP_OWNER="${TAP_OWNER:-life2you}"
TAP_REPO="${TAP_REPO:-homebrew-tap}"
WAIT_INTERVAL_SECONDS="${WAIT_INTERVAL_SECONDS:-5}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-900}"
SKIP_PUSH=0
SKIP_BREW=0
VERSION=""

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Required command not found: $cmd" >&2
    exit 1
  fi
}

show_help() {
  cat <<EOF
Usage: ./scripts/release-and-upgrade-local.sh [options] [version]

Options:
  --skip-push   Assume branch and tag have already been pushed.
  --skip-brew   Only wait for the tap update, do not run brew update/upgrade.
  --help        Show this help message.

Behavior:
  1. Detect the repo version and matching tag.
  2. Push the current branch and release tag unless --skip-push is used.
  3. Prefer gh for GitHub release checks when gh is installed and authenticated.
  4. Wait until life2you/homebrew-tap updates the formula to this version.
  5. Run brew update and brew upgrade/install for the local formula unless --skip-brew is used.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-push)
      SKIP_PUSH=1
      shift
      ;;
    --skip-brew)
      SKIP_BREW=1
      shift
      ;;
    --help)
      show_help
      exit 0
      ;;
    --)
      shift
      if [[ $# -gt 0 ]]; then
        VERSION="$1"
        shift
      fi
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        echo "Unexpected extra argument: $1" >&2
        exit 1
      fi
      VERSION="$1"
      shift
      ;;
  esac
done

if [[ $# -gt 0 ]]; then
  echo "Unexpected extra arguments: $*" >&2
  exit 1
fi

require_command git
require_command curl

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)}"
if [[ -z "$VERSION" ]]; then
  echo "Failed to detect version from Cargo.toml" >&2
  exit 1
fi

TAG="v$VERSION"
CURRENT_BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)"
if [[ "$CURRENT_BRANCH" == "HEAD" && "$SKIP_PUSH" != "1" ]]; then
  echo "Detached HEAD detected. Push manually or rerun with --skip-push after pushing the tag." >&2
  exit 1
fi

if ! git -C "$REPO_ROOT" rev-parse "$TAG^{}" >/dev/null 2>&1; then
  echo "Local tag $TAG does not exist. Create it first." >&2
  exit 1
fi

FORMULA_FILE=""
for candidate in \
  "$REPO_ROOT"/packaging/homebrew-tap/Formula/*.rb \
  "$REPO_ROOT"/homebrew/*.rb
do
  if [[ -f "$candidate" ]]; then
    FORMULA_FILE="$(basename "$candidate")"
    break
  fi
done

if [[ -z "$FORMULA_FILE" ]]; then
  echo "Unable to locate formula template in this repository." >&2
  exit 1
fi

FORMULA_NAME="${FORMULA_FILE%.rb}"
FORMULA_URL="https://raw.githubusercontent.com/${TAP_OWNER}/${TAP_REPO}/main/Formula/${FORMULA_FILE}"

GH_READY=0
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  GH_READY=1
  echo "Detected gh; will use it to check release visibility first."
else
  echo "gh not available or not authenticated; falling back to raw GitHub checks."
fi

if [[ "$SKIP_PUSH" != "1" ]]; then
  echo "Pushing branch ${CURRENT_BRANCH}..."
  git -C "$REPO_ROOT" push origin "$CURRENT_BRANCH"
  echo "Pushing tag ${TAG}..."
  git -C "$REPO_ROOT" push origin "$TAG"
fi

deadline=$(( $(date +%s) + TIMEOUT_SECONDS ))

if [[ "$GH_READY" == "1" ]]; then
  until gh release view "$TAG" --repo "${OWNER}/$(basename "$REPO_ROOT")" >/dev/null 2>&1; do
    if (( $(date +%s) >= deadline )); then
      echo "Timed out waiting for GitHub release $TAG to appear." >&2
      exit 1
    fi
    sleep "$WAIT_INTERVAL_SECONDS"
  done
fi

echo "Waiting for ${FORMULA_FILE} in ${TAP_OWNER}/${TAP_REPO} to update to ${VERSION}..."
while true; do
  if content="$(curl --fail --silent --show-error --location "$FORMULA_URL" 2>/dev/null)"; then
    if grep -Fq "version \"$VERSION\"" <<<"$content" || grep -Fq "/$TAG/" <<<"$content"; then
      break
    fi
  fi

  if (( $(date +%s) >= deadline )); then
    echo "Timed out waiting for Homebrew tap update: ${FORMULA_URL}" >&2
    exit 1
  fi

  sleep "$WAIT_INTERVAL_SECONDS"
done

echo "Homebrew tap updated to ${VERSION}."

if [[ "$SKIP_BREW" == "1" ]]; then
  exit 0
fi

require_command brew
echo "Running brew update..."
brew update
echo "Upgrading local formula ${FORMULA_NAME}..."
brew upgrade "life2you/tap/${FORMULA_NAME}" || brew install "life2you/tap/${FORMULA_NAME}"
echo "Local Homebrew package is now synchronized."
