//! How each locked version was published, and what to flag: a version published with a token after
//! its crate had started using Trusted Publishing, the way a stolen token would publish, and a
//! lockfile checksum that isn't the one crates.io has.

use serde::Serialize;

use crate::lockfile::Locked;
use crate::registry::{TrustPub, Version};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum How {
    TrustedPublishing { data: TrustPub },
    Token { user: Option<String> },
}

/// The crate's first version published with Trusted Publishing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Since {
    pub version: String,
    pub published: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub version: String,
    pub published: Option<String>,
    pub how: Option<How>,
    /// Set when this version came with a token after `since`.
    pub back_to_token: Option<Since>,
    /// Whether Cargo.lock's checksum is crates.io's; unknown when the lockfile has none.
    pub checksum_matches: Option<bool>,
    pub yanked: bool,
    pub error: Option<String>,
}

impl Entry {
    pub fn failed(locked: &Locked, error: String) -> Self {
        Entry {
            name: locked.name.clone(),
            version: locked.version.clone(),
            published: None,
            how: None,
            back_to_token: None,
            checksum_matches: None,
            yanked: false,
            error: Some(error),
        }
    }
}

/// Assesses one locked version against its crate's history. Publish times are crates.io's RFC 3339
/// timestamps in UTC, which sort as text.
pub fn assess(locked: &Locked, versions: &[Version]) -> Entry {
    let Some(this) = versions.iter().find(|v| v.num == locked.version) else {
        return Entry::failed(
            locked,
            format!("{} {} isn't on crates.io", locked.name, locked.version),
        );
    };
    let first_trusted = versions
        .iter()
        .filter(|v| v.trustpub_data.is_some())
        .min_by(|a, b| a.created_at.cmp(&b.created_at));
    let how = match &this.trustpub_data {
        Some(data) => How::TrustedPublishing { data: data.clone() },
        None => How::Token {
            user: this.published_by.as_ref().map(|u| u.login.clone()),
        },
    };
    let back_to_token = match (&this.trustpub_data, first_trusted) {
        (None, Some(first)) if first.created_at < this.created_at => Some(Since {
            version: first.num.clone(),
            published: first.created_at.clone(),
            source: first
                .trustpub_data
                .as_ref()
                .map(|d| format!("{} {}", d.provider, d.source()))
                .unwrap_or_default(),
        }),
        _ => None,
    };
    Entry {
        name: locked.name.clone(),
        version: locked.version.clone(),
        published: Some(this.created_at.clone()),
        how: Some(how),
        back_to_token,
        checksum_matches: locked.checksum.as_ref().map(|c| c == &this.checksum),
        yanked: this.yanked,
        error: None,
    }
}

/// 2 when something couldn't be checked, 1 when a version went back to a token or a checksum is wrong, else 0.
pub fn exit_code(entries: &[Entry]) -> i32 {
    if entries.iter().any(|e| e.error.is_some()) {
        2
    } else if entries
        .iter()
        .any(|e| e.back_to_token.is_some() || e.checksum_matches == Some(false))
    {
        1
    } else {
        0
    }
}

fn day(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

/// A summary for a person, with every package listed when `all` is set.
pub fn describe(entries: &[Entry], all: bool) -> String {
    let trusted = entries
        .iter()
        .filter(|e| matches!(e.how, Some(How::TrustedPublishing { .. })))
        .count();
    let token = entries
        .iter()
        .filter(|e| matches!(e.how, Some(How::Token { .. })))
        .count();
    let mut lines = vec![format!(
        "{} crates.io packages in Cargo.lock: {trusted} published with Trusted Publishing, {token} with a token",
        entries.len()
    )];
    for e in entries {
        if let Some(error) = &e.error {
            lines.push(format!("  ERROR {} {}: {error}", e.name, e.version));
        }
        if let (Some(since), Some(How::Token { user })) = (&e.back_to_token, &e.how) {
            let by = user
                .as_deref()
                .map(|u| format!(" by {u}"))
                .unwrap_or_default();
            lines.push(format!(
                "  FAIL  {} {} was published {} with a token{by}, after {} had used Trusted Publishing since {} ({}, from {})",
                e.name,
                e.version,
                day(e.published.as_deref().unwrap_or("?")),
                e.name,
                since.version,
                day(&since.published),
                since.source
            ));
        }
        if e.checksum_matches == Some(false) {
            lines.push(format!(
                "  FAIL  {} {}: Cargo.lock's checksum isn't the one crates.io has",
                e.name, e.version
            ));
        }
        if e.yanked {
            lines.push(format!("  note  {} {} is yanked", e.name, e.version));
        }
    }
    let checked = entries
        .iter()
        .filter(|e| e.checksum_matches.is_some())
        .count();
    let matching = entries
        .iter()
        .filter(|e| e.checksum_matches == Some(true))
        .count();
    lines.push(format!(
        "  checksums: {matching} of {checked} match crates.io"
    ));
    if all {
        for e in entries {
            let line = match &e.how {
                Some(How::TrustedPublishing { data }) => {
                    let at = data
                        .sha
                        .as_deref()
                        .map(|s| format!("@{}", s.get(..7).unwrap_or(s)))
                        .unwrap_or_default();
                    let run = data
                        .run_id
                        .as_deref()
                        .map(|r| format!(", run {r}"))
                        .unwrap_or_default();
                    format!(
                        "  trusted  {} {}  {} {}{at}{run}",
                        e.name,
                        e.version,
                        data.provider,
                        data.source()
                    )
                }
                Some(How::Token { user }) => format!(
                    "  token    {} {}  {}",
                    e.name,
                    e.version,
                    user.as_deref()
                        .map(|u| format!("by {u}"))
                        .unwrap_or_else(|| "publisher not recorded".into())
                ),
                None => continue,
            };
            lines.push(line);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::User;

    fn trusted(sha: &str) -> Option<TrustPub> {
        Some(TrustPub {
            provider: "github".into(),
            repository: Some("owner/repo".into()),
            project_path: None,
            run_id: Some("42".into()),
            sha: Some(sha.into()),
        })
    }
    fn version(num: &str, at: &str, trustpub: Option<TrustPub>) -> Version {
        Version {
            num: num.into(),
            created_at: format!("2026-{at}T00:00:00.000000Z"),
            checksum: format!("sum-{num}"),
            yanked: false,
            published_by: trustpub.is_none().then(|| User {
                login: "maintainer".into(),
            }),
            trustpub_data: trustpub,
        }
    }
    fn locked(version: &str, checksum: Option<&str>) -> Locked {
        Locked {
            name: "demo".into(),
            version: version.into(),
            checksum: checksum.map(String::from),
        }
    }

    /// 1.0 and 1.1 with a token, 1.2 with Trusted Publishing, then 1.3 with a token again and 1.4
    /// trusted. 0.9.9 was a backport, published with a token after 1.2.
    fn history() -> Vec<Version> {
        vec![
            version("1.4.0", "06-01", trusted("e")),
            version("1.3.0", "05-01", None),
            version("0.9.9", "04-15", None),
            version("1.2.0", "04-01", trusted("c")),
            version("1.1.0", "03-01", None),
            version("1.0.0", "02-01", None),
        ]
    }

    #[test]
    fn a_trusted_version_names_its_source() {
        let entry = assess(&locked("1.4.0", Some("sum-1.4.0")), &history());
        assert_eq!(
            entry.how,
            Some(How::TrustedPublishing {
                data: trusted("e").unwrap()
            })
        );
        assert_eq!(entry.back_to_token, None);
        assert_eq!(entry.checksum_matches, Some(true));
    }

    #[test]
    fn a_token_after_trusted_publishing_is_flagged_by_publish_time_not_semver() {
        for v in ["1.3.0", "0.9.9"] {
            let since = assess(&locked(v, None), &history()).back_to_token.expect(v);
            assert_eq!(
                (since.version.as_str(), since.source.as_str()),
                ("1.2.0", "github owner/repo")
            );
        }
    }

    #[test]
    fn tokens_before_trusted_publishing_began_are_fine() {
        for v in ["1.0.0", "1.1.0"] {
            let entry = assess(&locked(v, None), &history());
            assert_eq!(entry.back_to_token, None, "{v}");
            assert_eq!(
                entry.how,
                Some(How::Token {
                    user: Some("maintainer".into())
                })
            );
        }
    }

    #[test]
    fn a_crate_that_never_used_trusted_publishing_is_fine() {
        let history: Vec<_> = history()
            .into_iter()
            .map(|mut v| {
                v.trustpub_data = None;
                v
            })
            .collect();
        assert_eq!(assess(&locked("1.3.0", None), &history).back_to_token, None);
    }

    #[test]
    fn checksums_are_compared_when_the_lockfile_has_one() {
        assert_eq!(
            assess(&locked("1.1.0", Some("other")), &history()).checksum_matches,
            Some(false)
        );
        assert_eq!(
            assess(&locked("1.1.0", None), &history()).checksum_matches,
            None
        );
    }

    #[test]
    fn a_version_crates_io_doesnt_have_is_an_error() {
        let entry = assess(&locked("9.9.9", None), &history());
        assert_eq!(
            entry.error.as_deref(),
            Some("demo 9.9.9 isn't on crates.io")
        );
        assert_eq!(exit_code(&[entry]), 2);
    }

    #[test]
    fn exit_codes() {
        let fine = assess(&locked("1.4.0", Some("sum-1.4.0")), &history());
        let regressed = assess(&locked("1.3.0", None), &history());
        let tampered = assess(&locked("1.0.0", Some("nope")), &history());
        assert_eq!(exit_code(std::slice::from_ref(&fine)), 0);
        assert_eq!(exit_code(&[fine.clone(), regressed]), 1);
        assert_eq!(exit_code(&[fine, tampered]), 1);
        assert_eq!(exit_code(&[]), 0);
    }

    #[test]
    fn the_summary_names_each_problem() {
        let mut yanked = history();
        yanked[0].yanked = true;
        let entries = vec![
            assess(&locked("1.4.0", Some("sum-1.4.0")), &yanked),
            assess(&locked("1.3.0", Some("bad")), &yanked),
        ];
        let text = describe(&entries, true);
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "2 crates.io packages in Cargo.lock: 1 published with Trusted Publishing, 1 with a token",
                "  note  demo 1.4.0 is yanked",
                "  FAIL  demo 1.3.0 was published 2026-05-01 with a token by maintainer, after demo had used Trusted Publishing since 1.2.0 (2026-04-01, from github owner/repo)",
                "  FAIL  demo 1.3.0: Cargo.lock's checksum isn't the one crates.io has",
                "  checksums: 1 of 2 match crates.io",
                "  trusted  demo 1.4.0  github owner/repo@e, run 42",
                "  token    demo 1.3.0  by maintainer",
            ]
        );
    }
}
