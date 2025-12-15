#!/usr/bin/env bash

set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
git pull origin main
VERSION=$(yq -oy .package.version "$DIR"/cmha/Cargo.toml)
git log -n 1
git tag -fs "$VERSION" -m "$VERSION"
git tag -v "$VERSION"
git push origin "$(git tag -l '[0-9]*.[0-9]*.[0-9]*')"
