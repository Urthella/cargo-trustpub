//! A second opinion on Trusted Publishing from GitHub: the workflow run crates.io recorded must
//! exist in the repository it named and must have built the commit it named.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::tls::{TlsConfig, TlsProvider};

#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub head_sha: String,
    /// The workflow file, such as `.github/workflows/release.yml`.
    pub path: String,
    pub repository: Repository,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub full_name: String,
}

/// What GitHub says about the run behind a Trusted Publishing release.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RunCheck {
    /// The run is in the repository and built the commit crates.io recorded.
    Confirmed { workflow: String },
    /// The run built another commit.
    OtherCommit { built: String },
    /// The run belongs to another repository.
    OtherRepository { repository: String },
    /// GitHub has no such run; maintainers can delete runs, so this alone isn't a failure.
    NotFound,
}

impl RunCheck {
    pub fn fails(&self) -> bool {
        matches!(
            self,
            RunCheck::OtherCommit { .. } | RunCheck::OtherRepository { .. }
        )
    }
}

/// Compares a run with what crates.io recorded. Repository names are case-insensitive on GitHub.
pub fn compare(run: &Run, repository: &str, sha: &str) -> RunCheck {
    if !run.repository.full_name.eq_ignore_ascii_case(repository) {
        RunCheck::OtherRepository {
            repository: run.repository.full_name.clone(),
        }
    } else if run.head_sha != sha {
        RunCheck::OtherCommit {
            built: run.head_sha.clone(),
        }
    } else {
        RunCheck::Confirmed {
            workflow: run.path.clone(),
        }
    }
}

pub struct GitHub {
    api: String,
    agent: ureq::Agent,
    token: Option<String>,
}

impl GitHub {
    /// Uses `token` when given; public repositories' runs can be read without one, within
    /// GitHub's unauthenticated rate limit.
    pub fn new(api: &str, token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        GitHub {
            api: api.trim_end_matches('/').to_string(),
            agent,
            token,
        }
    }

    /// The run, or None if GitHub doesn't have it.
    pub fn run(&self, repository: &str, run_id: &str) -> Result<Option<Run>, String> {
        let url = format!("{}/repos/{repository}/actions/runs/{run_id}", self.api);
        let mut request = self
            .agent
            .get(&url)
            .header(
                "User-Agent",
                concat!("cargo-trustpub/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json");
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let mut response = request.call().map_err(|e| format!("{url}: {e}"))?;
        match response.status().as_u16() {
            200 => {
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| format!("{url}: {e}"))?;
                serde_json::from_str(&body)
                    .map(Some)
                    .map_err(|e| format!("{url}: unexpected response: {e}"))
            }
            404 => Ok(None),
            403 | 429 => Err(format!("{url}: rate limited by GitHub; set GITHUB_TOKEN")),
            status => Err(format!("{url}: HTTP {status}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sha: &str, repository: &str) -> Run {
        Run {
            head_sha: sha.into(),
            path: ".github/workflows/release.yml".into(),
            repository: Repository {
                full_name: repository.into(),
            },
        }
    }

    #[test]
    fn a_run_in_the_repository_that_built_the_commit_is_confirmed() {
        assert_eq!(
            compare(&run("abc", "Owner/Repo"), "owner/repo", "abc"),
            RunCheck::Confirmed {
                workflow: ".github/workflows/release.yml".into()
            }
        );
    }

    #[test]
    fn another_commit_or_repository_fails() {
        let other_commit = compare(&run("def", "owner/repo"), "owner/repo", "abc");
        assert_eq!(
            other_commit,
            RunCheck::OtherCommit {
                built: "def".into()
            }
        );
        let other_repository = compare(&run("abc", "someone/else"), "owner/repo", "abc");
        assert_eq!(
            other_repository,
            RunCheck::OtherRepository {
                repository: "someone/else".into()
            }
        );
        assert!(other_commit.fails() && other_repository.fails());
        assert!(
            !RunCheck::NotFound.fails()
                && !RunCheck::Confirmed {
                    workflow: String::new()
                }
                .fails()
        );
    }
}
