#!/usr/bin/env sh
# writes every generated test file into a folder (default: test-media) for trying by hand.
# the tests make the same files on their own, under target/tmp/test-media.
set -eu
cd "$(dirname "$0")/.."
cargo run --quiet -p vellum-media --example make_test_media -- "${1:-test-media}"
