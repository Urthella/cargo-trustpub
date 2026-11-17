# cargo-trustpub

How was each crates.io dependency in your `Cargo.lock` published? With
[Trusted Publishing](https://crates.io/docs/trusted-publishing) from CI, or with an API token, and
by whom? `cargo trustpub` asks crates.io about every locked version, and it fails in two cases:

- a locked version was published with a token after its crate had started using Trusted
  Publishing, which is what a release made with a stolen token would look like;
- a lockfile checksum isn't the one crates.io has.

## Why

Since July 2025, crates.io can take releases straight from a CI workflow, with no long-lived
token to steal. For every version, crates.io records how it arrived:

- **Trusted Publishing:** `trustpub_data`, with the repository, the workflow run and the commit;
- **a token:** `published_by`, the user whose token published it.

Once a crate has switched, a token release stands out. It may be a maintainer publishing by hand,
or an old branch without the workflow, or someone else's hands on a token.

How common is each? Surveying the 100 most downloaded crates on 2026-11-17, with
[`examples/survey.rs`](examples/survey.rs):

- 13 published their newest version with Trusted Publishing;
- 15 have used it for at least one version;
- 4 published with a token again after they had started: getrandom, rand_core, rand and
  cpufeatures. Most of those releases came from older branches. getrandom 0.2.17 came after
  Trusted Publishing began with 0.4.0-rc.0, and rand_core 0.1.1 came in September 2026, ten
  months after 0.10.0-rc-2 had used it.

## Quick start

Needs a Rust toolchain.

```sh
cargo install --git https://github.com/Urthella/cargo-trustpub
cd your-project
cargo trustpub
```

On a lockfile that pins rand 0.10.0, with the output it gives for real crates.io data:

```text
2 crates.io packages in Cargo.lock: 0 published with Trusted Publishing, 2 with a token
  FAIL  rand 0.10.0 was published 2026-02-08 with a token by dhardy, after rand had used Trusted Publishing since 0.10.0-rc.8 (2026-01-26, from github rust-random/rand)
  checksums: 2 of 2 match crates.io
```

`--all` adds a line for every package, such as these from ripgrep's lockfile:

```text
  trusted  cc 1.4.0  github rust-lang/cc-rs@413c4ec, run 30093615681
  token    bstr 1.13.0  by BurntSushi
```

Options:

- `--lockfile PATH` reads another lockfile than `./Cargo.lock`.
- `--json` prints every entry as JSON, Trusted Publishing details included.
- `--github` asks GitHub for a second opinion on each Trusted Publishing release from GitHub.
  The workflow run crates.io recorded must be in the repository it names and must have built
  the commit it names; the report then names the workflow file. A run that built another
  commit, or belongs to another repository, fails. A run GitHub no longer has is noted, since
  maintainers can delete runs. Public repositories' runs can be read without a token, up to 60
  an hour; with `GITHUB_TOKEN` or `GH_TOKEN` set, it's 5,000.
- `--contact EMAIL` adds an address to the User-Agent. crates.io's
  [data access policy](https://crates.io/data-access) asks API clients to say who they are.
- `--interval MS` spaces requests, 1,000 ms by default, which is the policy's limit; it's there
  for tests and mirrors.
- `--api URL` points at another crates.io API, such as a mirror.

Exit codes: 0 when nothing is flagged, 1 for a token release after Trusted Publishing or a
checksum that doesn't match, and 2 when a package couldn't be checked.

## What it does

1. **Reads `Cargo.lock`** and keeps the packages from crates.io's index, git or sparse. Git, path
   and other registries' packages are skipped.
2. **Fetches each crate's record** from `/api/v1/crates/NAME`, once per crate, a second apart. A
   429 is waited out.
3. **Judges each locked version** against the crate's whole history:
   - how it was published: the Trusted Publishing provider, repository, commit and run, or the
     token's user;
   - whether a version was published with Trusted Publishing *before* it, by publish time rather
     than semver, so a backport on an old branch counts too;
   - whether Cargo.lock's checksum is crates.io's;
   - whether it's yanked, which is noted but doesn't fail.
4. **With `--github`, asks GitHub about each run** at `/repos/OWNER/REPO/actions/runs/ID`. It
   compares the run's `head_sha` with the commit crates.io recorded, and the run's repository
   with the one it named, ignoring case as GitHub does.

## How it's checked

- **The rules on their own**, in the unit tests: a token release after Trusted Publishing,
  ordered by publish time, so a backport counts; tokens before it began are fine; a crate that
  never used it is fine; checksums, missing versions, exit codes and the summary's wording.
- **End to end against real records.** The binary runs against a fake crates.io serving the
  records of rand, jiff and itoa, saved by
  [`scripts/fetch-fixtures.sh`](scripts/fetch-fixtures.sh), with lockfiles built from their real
  checksums:
  - rand 0.10.0 fails, as published with dhardy's token after Trusted Publishing began;
  - jiff and rand's newest versions pass, with their commits and runs;
  - a tampered checksum fails;
  - an unknown crate or version exits 2;
  - a 429 is retried with `--contact` in the User-Agent, and requests keep the interval;
  - two versions of one crate share one request;
  - it runs as `cargo trustpub`, and bad arguments exit 2.
- **`--github` against real runs.** The same fake server answers for GitHub with the runs behind
  rand's and jiff's newest releases, saved from GitHub:
  - both runs are confirmed, the workflow file is named, and `GITHUB_TOKEN` is sent;
  - a run that built another commit, or belongs to another repository, fails;
  - a missing run is a note, a rate limit exits 2, and without the flag GitHub isn't asked.
- **Mutation testing.** Twenty-six deliberate bugs were planted one at a time, and the tests caught
  every one.
- **Live.** On its own lockfile, 66 crates.io packages: 7 published with Trusted Publishing, and
  all 66 checksums match crates.io. On ripgrep's, 50 packages: 3 trusted, and 50 of 50 checksums.
  Neither has a token release after Trusted Publishing. With `--github`, GitHub confirmed all 7
  and all 3 runs built the commits crates.io recorded.

## Limits

- It trusts what crates.io's API says. A checksum match shows the lockfile agrees with crates.io,
  not that crates.io is right.
- Versions from before crates.io recorded publishers say "publisher not recorded".
- `--github` checks GitHub's releases only; GitLab's Trusted Publishing records aren't
  cross-checked.
- At one request a second, a lockfile with 600 crates takes ten minutes.
- A flag is a reason to look, not a verdict: most token releases after Trusted Publishing among
  the top crates are maintenance releases on old branches.

## License

MIT
