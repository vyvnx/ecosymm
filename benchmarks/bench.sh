#!/usr/bin/env bash
# wall clock and peak population vs founders per species. run from the repo root.
set -euo pipefail
cargo build --release
printf '%22s  %8s  %8s  %s\n' "population per species" "epochs" "seconds" "peak total population"
for n in 125 500 2000; do
  out=$(./target/release/ecosym --seed 1234 --population-per-species "$n" --epochs 200)
  secs=$(awk '/wall clock/ { print $3 }' <<<"$out")
  peak=$(awk '$1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { if ($2 > m) m = $2 } END { print m }' <<<"$out")
  printf '%22s  %8s  %8s  %s\n' "$n" 200 "$secs" "$peak"
done
