#!/usr/bin/env bash
#
# Install the repository git hooks: points core.hooksPath at .githooks.
# Usage: ./scripts/setup-hooks.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

chmod +x .githooks/* 2>/dev/null || true
git config core.hooksPath .githooks

echo "✓ git hooks installed (core.hooksPath = .githooks)"
