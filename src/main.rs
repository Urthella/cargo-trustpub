//! cargo-trustpub: how was each crates.io dependency in Cargo.lock published? With Trusted
//! Publishing from CI, or with an API token, and by whom. Exits 1 when a locked version was
//! published with a token after its crate had started using Trusted Publishing, or when a
//! lockfile checksum isn't crates.io's, and 2 when something couldn't be checked. With
//! `--github`, it also asks GitHub whether each Trusted Publishing run built the commit crates.io
//! recorded, and exits 1 if not.

use cargo_trustpub::github::{self, GitHub, RunCheck};
use cargo_trustpub::report::How;
use cargo_trustpub::{lockfile, registry, report};

use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: cargo trustpub [--lockfile PATH] [--all] [--json] [--github] [--contact EMAIL] [--api URL] [--github-api URL] [--interval MS]";

struct Options {
    lockfile: String,
    all: bool,
    json: bool,
    github: bool,
    contact: Option<String>,
    api: String,
    github_api: String,
    interval: Duration,
}

fn options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        lockfile: "Cargo.lock".into(),
        all: false,
        json: false,
        github: false,
        contact: None,
        api: "https://crates.io/api/v1".into(),
        github_api: "https://api.github.com".into(),
        interval: Duration::from_secs(1),
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--lockfile" => options.lockfile = value("--lockfile")?,
            "--all" => options.all = true,
            "--json" => options.json = true,
            "--github" => options.github = true,
            "--contact" => options.contact = Some(value("--contact")?),
            "--api" => options.api = value("--api")?,
            "--github-api" => options.github_api = value("--github-api")?,
            "--interval" => {
                options.interval = Duration::from_millis(
                    value("--interval")?
                        .parse()
                        .map_err(|_| "--interval takes milliseconds".to_string())?,
                )
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    // Run as `cargo trustpub`, cargo passes the subcommand's name first.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "trustpub") {
        args.remove(0);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let options = match options(args.into_iter()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let locked = match std::fs::read_to_string(&options.lockfile)
        .map_err(|e| format!("{}: {e}", options.lockfile))
        .and_then(|text| lockfile::parse(&text))
    {
        Ok(locked) => locked,
        Err(message) => {
            eprintln!("cargo-trustpub: {message}");
            return ExitCode::from(2);
        }
    };
    let mut registry =
        registry::Registry::new(&options.api, options.contact.as_deref(), options.interval);
    let mut entries = Vec::new();
    let mut previous: Option<(String, Result<Vec<registry::Version>, String>)> = None;
    for package in &locked {
        // Packages are sorted by name, so two versions of one crate share a request.
        if previous
            .as_ref()
            .is_none_or(|(name, _)| name != &package.name)
        {
            previous = Some((package.name.clone(), registry.versions(&package.name)));
        }
        let entry = match &previous.as_ref().expect("set above").1 {
            Ok(versions) => report::assess(package, versions),
            Err(error) => report::Entry::failed(package, error.clone()),
        };
        entries.push(entry);
    }
    if options.github {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok();
        let github = GitHub::new(&options.github_api, token);
        for entry in &mut entries {
            if let Some(How::TrustedPublishing { data }) = &entry.how
                && data.provider == "github"
                && let (Some(repository), Some(run), Some(sha)) =
                    (&data.repository, &data.run_id, &data.sha)
            {
                match github.run(repository, run) {
                    Ok(Some(found)) => {
                        entry.github = Some(github::compare(&found, repository, sha))
                    }
                    Ok(None) => entry.github = Some(RunCheck::NotFound),
                    Err(error) => entry.error = Some(format!("GitHub: {error}")),
                }
            }
        }
    }
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).expect("entries serialize")
        );
    } else {
        println!("{}", report::describe(&entries, options.all));
    }
    ExitCode::from(report::exit_code(&entries) as u8)
}
