use std::collections::HashMap;

use color_eyre::eyre::{eyre, Context};
use color_eyre::Result;
use lazy_static::lazy_static;
use serde::Deserialize;
use tracing::trace;
use url::Url;

use crate::version::LenientVersion;
use crate::version_checker::{get_new_version_filename, VersionCheck};
use crate::CLIENT;

lazy_static! {
    static ref BASE_URL: Url = "https://pypi.org/pypi/"
        .parse()
        .expect("error parsing pypi url");
}

pub struct PyPi {
    base_url: Url,
    current_version: LenientVersion,
    project_name: String,
    remote_version: Option<LenientVersion>,
    remote_url: Option<String>,
}

#[derive(Deserialize)]
struct Project {
    releases: HashMap<LenientVersion, Vec<Release>>,
}

#[derive(Deserialize)]
struct Release {
    filename: String,
    url: String,
}

impl PyPi {
    pub fn new(current_download_url: &Url, current_version: LenientVersion) -> Result<Self> {
        PyPi::with_pypi_url(&*BASE_URL, current_download_url, current_version)
    }

    pub(crate) fn with_pypi_url(
        base_url: &Url,
        current_download_url: &Url,
        current_version: LenientVersion,
    ) -> Result<Self> {
        let project_name = current_download_url
            .path()
            .split('/')
            .nth(4)
            .ok_or_else(|| {
                eyre!(
                    "failed to get project from current url {}",
                    current_download_url
                )
            })?;
        Ok(Self {
            base_url: base_url.clone(),
            current_version,
            project_name: project_name.to_string(),
            remote_version: None,
            remote_url: None,
        })
    }

    async fn do_fetch_last_version(&mut self, file_template: &str) -> color_eyre::Result<()> {
        let url = self
            .base_url
            .join(&format!("{}/json", self.project_name))
            .wrap_err("failed to get pypi url")?;
        trace!(message = "fetching pypi version", file_template = file_template, url = %url);
        let response = CLIENT
            .get(url)
            .send()
            .await
            .wrap_err("failed to get latest version")?;
        let project: Project = response.json().await?;

        if let Some((version, release)) = project.latest_version() {
            self.remote_version = Some(version.clone());
            self.remote_url = self.get_matching_release(release, file_template);
        }
        Ok(())
    }

    fn get_matching_release(&self, releases: &[Release], file_template: &str) -> Option<String> {
        let remote_version = self.remote_version.as_ref()?;
        let expected_template =
            get_new_version_filename(file_template, &self.current_version, remote_version);
        releases
            .iter()
            .filter(|release| release.filename == expected_template)
            .map(|release| release.url.clone())
            .next()
    }
}

#[async_trait::async_trait]
impl VersionCheck for PyPi {
    fn checker_name(&self) -> &'static str {
        "pypi"
    }

    async fn fetch_last_version(&mut self, file_template: &str) -> color_eyre::Result<()> {
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

impl Project {
    fn latest_version(&self) -> Option<(&LenientVersion, &Vec<Release>)> {
        let mut versions = self
            .releases
            .iter()
            .filter_map(|(v, r)| {
                if v.inner().pre.is_empty() {
                    Some((v, r))
                } else {
                    None
                }
            })
            .collect::<Vec<(&LenientVersion, &Vec<Release>)>>();
        versions.sort_by(|(a, _), (b, _)| a.cmp(b));
        versions.last().cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::env::set_var;
    use std::{fs};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::setup_error_handlers;
    use crate::version::LenientVersion;
    use crate::version_checker::pypi::PyPi;
    use crate::version_checker::VersionCheck;

    #[tokio::test]
    async fn fetch_latest_version() {
        set_var("RUST_LOG", "aur_autoupdater=trace");
        setup_error_handlers().ok();
        let response = fs::read("tests/fixtures/pypi.json").unwrap();
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/pypi/ConfigUpdater/json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&*response))
            .expect(1)
            .mount(&mock_server)
            .await;

        let current_version = LenientVersion::parse("2.0").unwrap();
        let base_uri = format!("{}/pypi/", mock_server.uri());
        let download_url = "https://files.pythonhosted.org/packages/source/C/ConfigUpdater/ConfigUpdater-2.0.tar.gz".parse().unwrap();
        let mut pypi: Box<dyn VersionCheck> = Box::new(
            PyPi::with_pypi_url(
                &base_uri.parse().unwrap(),
                &download_url,
                current_version.clone(),
            )
            .unwrap(),
        );
        pypi.fetch_last_version("ConfigUpdater-2.0.tar.gz")
            .await
            .unwrap();

        let remote_version = LenientVersion::parse("3.0.1").unwrap();
        mock_server.verify().await;

        assert_eq!(pypi.get_current_version(), &current_version);
        assert_eq!(pypi.get_remote_version(), Some(&remote_version));
        assert_eq!(pypi.get_download_url(), Some("https://files.pythonhosted.org/packages/9b/0e/0e730b2b3691f8374a74833a48b90616eb4de61f197d924cebd8d2e07d00/ConfigUpdater-3.0.1.tar.gz"));
        assert!(pypi.has_newer_version());
    }
}
