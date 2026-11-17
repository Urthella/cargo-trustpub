//! How the most downloaded crates on crates.io publish: whether their newest version came through
//! Trusted Publishing, whether any version did, and which ones published with a token again after
//! they had started. It judges each version the way `cargo trustpub` judges a locked one.
//!
//!   cargo run --release --example survey -- [COUNT] [--contact EMAIL]

use std::time::Duration;

use cargo_trustpub::lockfile::Locked;
use cargo_trustpub::registry::Registry;
use cargo_trustpub::report::{How, assess};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut count = 100;
    let mut contact = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--contact" => contact = args.next(),
            n => count = n.parse().map_err(|_| format!("not a count: {n}"))?,
        }
    }
    let mut registry = Registry::new(
        "https://crates.io/api/v1",
        contact.as_deref(),
        Duration::from_secs(1),
    );
    let names = registry.most_downloaded(count)?;
    let (mut newest_trusted, mut ever_trusted) = (0, 0);
    let mut went_back = Vec::new();
    for name in &names {
        let versions = registry.versions(name)?;
        let judged: Vec<_> = versions
            .iter()
            .map(|v| {
                let locked = Locked {
                    name: name.clone(),
                    version: v.num.clone(),
                    checksum: None,
                };
                (v, assess(&locked, &versions))
            })
            .collect();
        let newest = judged
            .iter()
            .max_by(|a, b| a.0.created_at.cmp(&b.0.created_at));
        if newest.is_some_and(|(_, e)| matches!(e.how, Some(How::TrustedPublishing { .. }))) {
            newest_trusted += 1;
        }
        if versions.iter().any(|v| v.trustpub_data.is_some()) {
            ever_trusted += 1;
        }
        let back: Vec<_> = judged
            .iter()
            .filter(|(_, e)| e.back_to_token.is_some())
            .collect();
        if let Some((last, entry)) = back
            .iter()
            .max_by(|a, b| a.0.created_at.cmp(&b.0.created_at))
        {
            let since = entry.back_to_token.as_ref().expect("filtered");
            let by = last
                .published_by
                .as_ref()
                .map(|u| u.login.as_str())
                .unwrap_or("?");
            went_back.push(format!(
                "  {name}: {} token release{} after Trusted Publishing began with {} ({}), the latest {} on {} by {by}",
                back.len(),
                if back.len() == 1 { "" } else { "s" },
                since.version,
                &since.published[..10],
                last.num,
                &last.created_at[..10]
            ));
        }
    }
    println!("the {} most downloaded crates on crates.io:", names.len());
    println!("  {newest_trusted} published their newest version with Trusted Publishing");
    println!("  {ever_trusted} have published at least one version with it");
    println!(
        "  {} published with a token again after they had started:",
        went_back.len()
    );
    for line in went_back {
        println!("{line}");
    }
    Ok(())
}
