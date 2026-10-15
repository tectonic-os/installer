#!/bin/bash
set -euo pipefail

duration="${1:-60}"
case "$duration" in
    "" | *[!0-9]*)
        echo "duration must be a whole number of seconds" >&2
        exit 2
        ;;
esac

fuzz_dir="$(dirname -- "$(realpath -- "$0")")"
mkdir -p "$fuzz_dir/target"
cargo +nightly-2026-10-14 metadata --manifest-path "$fuzz_dir/Cargo.toml" \
    --format-version 1 --locked > /dev/null
targets="$(cargo +nightly-2026-10-14 fuzz list --fuzz-dir "$fuzz_dir")"

# Mutated input may contain control bytes. Keep libFuzzer's raw stream out of
# routine terminal and CI output; a failure artifact carries it for diagnosis.
while IFS= read -r target; do
    [ -n "$target" ] || continue
    corpus="$(mktemp -d "$fuzz_dir/target/corpus-${target}.XXXXXX")"
    cp -a "$fuzz_dir/corpus/$target/." "$corpus/"
    log="$fuzz_dir/target/${target}.log"
    if cargo +nightly-2026-10-14 fuzz run --fuzz-dir "$fuzz_dir" "$target" "$corpus" -- \
        -max_total_time="$duration" \
        -max_len=65536 \
        -timeout=10 > "$log" 2>&1; then
        printf 'fuzz target passed: %s (%ss)\n' "$target" "$duration"
    else
        printf 'fuzz target failed: %s; raw output is in %s\n' "$target" "$log" >&2
        exit 1
    fi
done <<< "$targets"
