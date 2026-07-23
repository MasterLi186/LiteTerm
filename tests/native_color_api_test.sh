#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if rg -n 'Color32::from_rgba_premultiplied' \
    "$ROOT/native-prototype/src" --glob '*.rs'; then
    echo "Native UI 仍在使用预乘 Alpha API" >&2
    exit 1
fi
