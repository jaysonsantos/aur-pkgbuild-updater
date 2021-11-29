use async_trait::async_trait;
use color_eyre::{eyre::eyre, Result};

use serde::Deserialize;
use tracing::{debug, instrument};
use url::Url;

use super::VersionCheck;
use crate::version::LenientVersion;
use crate::{package::VERSION_PLACEHOLDER, CLIENT};

#[derive(Debug)]
pub struct Github {
    github_base_url: String,
    organization: String,
    repository: String,
    current_version: LenientVersion,
    remote_version: Option<LenientVersion>,
    remote_url: Option<String>,
}

impl Github {
    pub fn new(url: &Url, current_version: LenientVersion) -> Result<Self> {
        Self::with_default_github_url(url, current_version)
    }

    fn with_default_github_url(url: &Url, current_version: LenientVersion) -> Result<Self> {
        Self::with_github_url(url, current_version, "https://api.github.com".to_string())
    }

    fn with_github_url(
        url: &Url,
        current_version: LenientVersion,
        github_base_url: String,
    ) -> Result<Self> {
        let mut path = url.path().split('/');
        let organization = path
            .nth(1)
            .ok_or_else(|| eyre!("failed to get organization from {:?}", &url))?
            .to_string();

        let repository = path
            .next()
            .ok_or_else(|| eyre!("failed to get repository from {:?}", &url))?
            .to_string();

        Ok(Self {
            github_base_url,
            organization,
            repository,
            current_version,
            remote_version: None,
            remote_url: None,
        })
    }

    #[instrument]
    async fn do_fetch_last_version(&mut self, file_template: &str) -> Result<()> {
        let releases_url = format!(
            "{}/repos/{}/{}/releases",
            self.github_base_url, self.organization, self.repository
        );
        let response = CLIENT.get(&releases_url).send().await?;
        let releases: Vec<Release> = response.json().await?;
        let mut latest_version: Option<LenientVersion> = None;
        let mut download_url: Option<String> = None;

        debug!("found {} release", releases.len());

        for release in &releases {
            if let Ok(tag_name) = LenientVersion::parse(&release.tag_name).as_ref() {
                debug!("checking tag {}", tag_name);
                for asset in &release.assets {
                    let file_name =
                        file_template.replace(VERSION_PLACEHOLDER, &tag_name.to_string());

                    if asset.browser_download_url.contains(&file_name) {
                        if let Some(current_latest_version) = latest_version.as_ref() {
                            if tag_name > current_latest_version {
                                latest_version = Some(tag_name.clone());
                                download_url = Some(asset.browser_download_url.clone());
                            }
                        } else {
                            latest_version = Some(tag_name.clone());
                            download_url = Some(asset.browser_download_url.clone());
                        }
                    }
                }
            }
        }

        if let Some(latest_version) = latest_version {
            self.remote_version = Some(latest_version);
            self.remote_url = download_url;
            return Ok(());
        }

        debug!("Falling back to tags as no releases lead to a newer version");
        let tags_url = format!(
            "{}/repos/{}/{}/tags",
            self.github_base_url, self.organization, self.repository
        );
        let response = CLIENT.get(&tags_url).send().await?;
        let tags: Vec<Tag> = response.json().await?;
        debug!("found {} tags", tags.len());

        for tag in &tags {
            if let Ok(tag_name) = LenientVersion::parse(&tag.name).as_ref() {
                debug!("checking tag {}", tag_name);
                if let Some(current_latest_version) = latest_version.as_ref() {
                    if tag_name > current_latest_version {
                        latest_version = Some(tag_name.clone());
                        download_url = Some(tag.get_download_url(self));
                    }
                } else {
                    latest_version = Some(tag_name.clone());
                    download_url = Some(tag.get_download_url(self));
                }
            }
        }

        if let Some(latest_version) = latest_version {
            self.remote_version = Some(latest_version);
            self.remote_url = download_url;
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Tag {
    name: String,
}

impl Tag {
    fn get_download_url(&self, github: &Github) -> String {
        format!(
            "https://github.com/{}/{}/archive/refs/tags/{}.tar.gz",
            github.organization, github.repository, self.name
        )
    }
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[async_trait]
impl VersionCheck for Github {
    fn checker_name(&self) -> &'static str {
        "github"
    }

    async fn fetch_last_version(&mut self, file_template: &str) -> Result<()> {
        self.do_fetch_last_version(file_template).await
    }

    fn get_current_version(&self) -> &LenientVersion {
        &self.current_version
    }

    fn get_remote_version(&self) -> Option<&LenientVersion> {
        self.remote_version.as_ref()
    }

    fn get_download_url(&self) -> Option<&str> {
        self.remote_url.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::env::set_var;

    use serde_json::json;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use super::{Github, VersionCheck};
    use crate::version::LenientVersion;
    use crate::{package::VERSION_PLACEHOLDER, setup_error_handlers};

    #[tokio::test]
    async fn fetch_last_version() {
        set_var("RUST_LOG", "aur_autoupdater=trace");
        setup_error_handlers().ok();
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/jaysonsantos/mambembe/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "tag_name": "0.1.1",
                "assets": [{
                    "name": "mambembe-cli-with-keyring-0.1.1-x86_64-unknown-linux-gnu.tar.gz",
                    "browser_download_url": "https://github.com/jaysonsantos/mambembe/releases/download/0.1.1/mambembe-cli-with-keyring-0.1.1-x86_64-unknown-linux-gnu.tar.gz"
                }]
            }])))
            .mount(&mock_server)
            .await;
        let mut github: Box<dyn VersionCheck> = Box::new(
            Github::with_github_url(
                &"https://github.com/jaysonsantos/mambembe/releases/tag/0.1.0"
                    .parse()
                    .unwrap(),
                LenientVersion::parse("0.1.0").unwrap(),
                mock_server.uri(),
            )
            .unwrap(),
        );

        let current_version = LenientVersion::parse("0.1.0").unwrap();
        let remote_version = LenientVersion::parse("0.1.1").unwrap();

        github.fetch_last_version(&format!("https://github.com/jaysonsantos/mambembe/releases/download/{0}/mambembe-cli-with-keyring-{0}-x86_64-unknown-linux-gnu.tar.gz", VERSION_PLACEHOLDER)).await.unwrap();
        assert!(github.has_newer_version());
        assert_eq!(github.get_current_version(), &current_version);
        assert_eq!(github.get_remote_version().unwrap(), &remote_version);
    }
}
