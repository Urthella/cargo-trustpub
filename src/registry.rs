//! crates.io's record of every version of a crate: when and how it was published. The data access
//! policy asks for at most one request per second and a User-Agent that names the client, so
//! requests are spaced out and `--contact` can add an address to the User-Agent.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ureq::tls::{TlsConfig, TlsProvider};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    pub num: String,
    pub created_at: String,
    pub checksum: String,
    #[serde(default)]
    pub yanked: bool,
    pub published_by: Option<User>,
    /// Set when the version was published with Trusted Publishing.
    pub trustpub_data: Option<TrustPub>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct User {
    pub login: String,
}

/// The CI identity crates.io verified: for GitHub, the repository, the workflow run and the commit.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TrustPub {
    pub provider: String,
    pub repository: Option<String>,
    pub project_path: Option<String>,
    pub run_id: Option<String>,
    pub sha: Option<String>,
}

impl TrustPub {
    /// Where the publish came from: a GitHub repository, or a GitLab project.
    pub fn source(&self) -> &str {
        self.repository
            .as_deref()
            .or(self.project_path.as_deref())
            .unwrap_or("?")
    }
}

#[derive(Deserialize)]
struct CrateResponse {
    versions: Vec<Version>,
}

pub struct Registry {
    api: String,
    agent: ureq::Agent,
    user_agent: String,
    interval: Duration,
    last: Option<Instant>,
}

const ATTEMPTS: u32 = 3;

impl Registry {
    pub fn new(api: &str, contact: Option<&str>, interval: Duration) -> Self {
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
        let version = env!("CARGO_PKG_VERSION");
        let user_agent = match contact {
            Some(contact) => format!("cargo-trustpub/{version} ({contact})"),
            None => format!("cargo-trustpub/{version}"),
        };
        Registry {
            api: api.trim_end_matches('/').to_string(),
            agent,
            user_agent,
            interval,
            last: None,
        }
    }

    /// Every version of the crate, as crates.io records it.
    pub fn versions(&mut self, name: &str) -> Result<Vec<Version>, String> {
        let url = format!("{}/crates/{name}", self.api);
        let body = self
            .get(&url)?
            .ok_or_else(|| format!("{name} isn't on crates.io"))?;
        let parsed: CrateResponse =
            serde_json::from_str(&body).map_err(|e| format!("{url}: unexpected response: {e}"))?;
        Ok(parsed.versions)
    }

    /// The names of the `count` most downloaded crates, at most 100.
    pub fn most_downloaded(&mut self, count: usize) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Listing {
            crates: Vec<Named>,
        }
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        let url = format!(
            "{}/crates?sort=downloads&per_page={}",
            self.api,
            count.min(100)
        );
        let body = self.get(&url)?.ok_or_else(|| format!("{url}: HTTP 404"))?;
        let listing: Listing =
            serde_json::from_str(&body).map_err(|e| format!("{url}: unexpected response: {e}"))?;
        Ok(listing.crates.into_iter().map(|c| c.name).collect())
    }

    /// A GET, spaced from the last one and retried after a 429; None for a 404.
    fn get(&mut self, url: &str) -> Result<Option<String>, String> {
        for attempt in 1..=ATTEMPTS {
            if let Some(last) = self.last {
                sleep(self.interval.saturating_sub(last.elapsed()));
            }
            self.last = Some(Instant::now());
            let mut response = self
                .agent
                .get(url)
                .header("User-Agent", &self.user_agent)
                .call()
                .map_err(|e| format!("{url}: {e}"))?;
            let status = response.status().as_u16();
            if status == 429 && attempt < ATTEMPTS {
                let wait = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5u64);
                sleep(Duration::from_secs(wait.min(30)));
                continue;
            }
            if status == 404 {
                return Ok(None);
            }
            if status != 200 {
                return Err(format!("{url}: HTTP {status}"));
            }
            let body = response
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_string()
                .map_err(|e| format!("{url}: {e}"))?;
            return Ok(Some(body));
        }
        Err(format!("{url}: rate limited"))
    }
}
