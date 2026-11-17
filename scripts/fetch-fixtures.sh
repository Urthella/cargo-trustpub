#!/bin/sh
# Saves crates.io's records of a few crates for the tests, one request a second as crates.io's
# data access policy asks: rand, which went back to a token for two releases after it started
# using Trusted Publishing; jiff, which switched and stayed; and itoa, which publishes with a token.
set -eu
cd "$(dirname "$0")/../tests/fixtures"
for crate in rand jiff itoa; do
  curl -sf -A "cargo-trustpub fixtures" "https://crates.io/api/v1/crates/$crate" -o "$crate.json"
  sleep 1
done

# The GitHub runs behind rand's and jiff's newest Trusted Publishing releases, for --github.
# GitHub allows 60 unauthenticated requests an hour; set GITHUB_TOKEN to go past that.
for crate in rand jiff; do
  pair=$(grep -o '"repository":"[^"]*","run_id":"[0-9]*"' "$crate.json" | head -1)
  repo=$(echo "$pair" | sed 's/.*"repository":"\([^"]*\)".*/\1/')
  run=$(echo "$pair" | sed 's/.*"run_id":"\([0-9]*\)".*/\1/')
  curl -sf ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} -H "Accept: application/vnd.github+json" -A "cargo-trustpub fixtures" \
    "https://api.github.com/repos/$repo/actions/runs/$run" -o "github-run-$run.json"
  echo "$crate: $repo run $run"
done
ls -l
