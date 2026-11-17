//! The crates.io packages a Cargo.lock pins, with the checksums it recorded for them. Git, path and
//! other registries' packages are left out: crates.io can't say how they were published.

use serde::Deserialize;

/// crates.io's index, as a lockfile names it: the git index or the sparse one.
pub const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Locked {
    pub name: String,
    pub version: String,
    pub checksum: Option<String>,
}

#[derive(Deserialize)]
struct Lockfile {
    #[serde(default)]
    package: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

pub fn parse(text: &str) -> Result<Vec<Locked>, String> {
    let lock: Lockfile =
        toml::from_str(text).map_err(|e| format!("not a Cargo.lock: {}", e.message()))?;
    let mut locked: Vec<Locked> = lock
        .package
        .into_iter()
        .filter(|p| p.source.as_deref().is_some_and(|s| CRATES_IO.contains(&s)))
        .map(|p| Locked {
            name: p.name,
            version: p.version,
            checksum: p.checksum,
        })
        .collect();
    locked.sort();
    locked.dedup();
    Ok(locked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_crates_io_packages_from_either_index() {
        let text = r#"
version = 4

[[package]]
name = "rand"
version = "0.10.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aa"

[[package]]
name = "jiff"
version = "1.0.0"
source = "sparse+https://index.crates.io/"
checksum = "bb"

[[package]]
name = "mine"
version = "0.1.0"

[[package]]
name = "forked"
version = "2.0.0"
source = "git+https://github.com/someone/forked?branch=main#0123456"

[[package]]
name = "internal"
version = "1.0.0"
source = "sparse+https://registry.example.com/index/"
checksum = "cc"
"#;
        let locked = parse(text).unwrap();
        assert_eq!(
            locked,
            vec![
                Locked {
                    name: "jiff".into(),
                    version: "1.0.0".into(),
                    checksum: Some("bb".into())
                },
                Locked {
                    name: "rand".into(),
                    version: "0.10.4".into(),
                    checksum: Some("aa".into())
                },
            ]
        );
    }

    #[test]
    fn two_versions_of_one_crate_are_both_kept() {
        let text = r#"
[[package]]
name = "rand"
version = "0.8.5"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "rand"
version = "0.10.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let versions: Vec<_> = parse(text)
            .unwrap()
            .into_iter()
            .map(|l| l.version)
            .collect();
        assert_eq!(versions, ["0.10.4", "0.8.5"]);
    }

    #[test]
    fn refuses_what_isnt_toml() {
        assert!(
            parse("[[package]\nname =")
                .unwrap_err()
                .starts_with("not a Cargo.lock")
        );
    }
}
