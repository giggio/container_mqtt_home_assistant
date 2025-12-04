#!/usr/bin/env bash

set -euo pipefail

git pull origin main
VERSION=$(yq -oy .package.version cmha/Cargo.toml)
git log -n 1
git tag -fs "$VERSION" -m "$VERSION"
git tag -v "$VERSION"
git push origin "$(git tag -l '[0-9]*.[0-9]*.[0-9]*')"
