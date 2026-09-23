#!/usr/bin/env bash
# Build the fastty-wasm crate and refresh web/pkg with the new artifacts.
# The gateway embeds web/pkg at compile time (src/gateway.rs include_str!/include_bytes!),
# so run this after any change to crates/fastty-wasm and commit the result.
set -euo pipefail

cd "$(dirname "$0")/crates/fastty-wasm"

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "wasm-pack not found. Install it with: cargo install wasm-pack" >&2
    exit 1
fi

wasm-pack build --target web --release --out-dir ../../web/pkg

# wasm-pack drops a .gitignore that would hide new pkg artifacts from git.
rm -f ../../web/pkg/.gitignore

echo "web/pkg refreshed. Commit the result so release builds embed the new wasm."
