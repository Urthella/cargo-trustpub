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
ls -l
