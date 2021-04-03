pub mod github;

use async_trait::async_trait;
use color_eyre::{
    eyre::{eyre, WrapErr},
    Result,
};
use semver::Version;
use url::Url;

use self::github::Github;
#[cfg(test)]
use self::tests::TestServer;

#[async_trait]
pub trait VersionCheck {
    fn checker_name(&self) -> &'static str;
    async fn fetch_last_version(&mut self, file_template: &str) -> Result<()>;
    fn get_current_version(&self) -> &Version;
    fn get_remote_version(&self) -> Option<&Version>;
    fn has_newer_version(&self) -> bool {
        if let Some(remote_version) = self.get_remote_version() {
            return remote_version > self.get_current_version();
        }
        false
    }

    fn get_download_url(&self) -> Option<&str>;
}

pub fn get_version_checker(url: &str, current_version: Version) -> Result<Box<dyn VersionCheck>> {
    let parsed_url = Url::parse(url).wrap_err_with(|| format!("failed to parse url {:?}", url))?;
    get_version_checker_from_parsed_url(&parsed_url, current_version)
        .wrap_err_with(|| format!("failed to find a checker for url {:?}", url))
}

fn get_version_checker_from_parsed_url(
    url: &Url,
    current_version: Version,
) -> Result<Box<dyn VersionCheck>> {
    match url.domain() {
        Some("github.com") => Ok(Box::new(Github::new(&url, current_version)?)),
        #[cfg(test)]
        Some("aur-test.localtest.me") => Ok(Box::new(TestServer::new())),
        e => Err(eyre!(
            "version checker not implemented for domain {:?} yet",
            e
        )),
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use color_eyre::Result;
    use semver::Version;

    use super::{get_version_checker, VersionCheck};

    pub struct TestServer {
        current_version: Version,
        remote_version: Version,
        download_url: String,
    }

    impl TestServer {
        pub fn new() -> Self {
            Self {
                current_version: Version::new(0, 1, 0),
                remote_version: Version::new(0, 1, 1),
                download_url: "http://aur-test.localtest.me:43987/0.1.1/test-package-0.1.1.tar.gz"
                    .to_owned(),
            }
        }
    }
    #[async_trait]
    impl VersionCheck for TestServer {
        fn checker_name(&self) -> &'static str {
            "test-server"
        }

        async fn fetch_last_version(&mut self, _file_template: &str) -> Result<()> {
            Ok(())
        }

        fn get_current_version(&self) -> &Version {
            &self.current_version
        }

        fn get_remote_version(&self) -> Option<&Version> {
            Some(&self.remote_version)
        }

        fn get_download_url(&self) -> Option<&str> {
            Some(self.download_url.as_str())
        }
    }

    #[test]
    fn test_get_version_checker() {
        let github = get_version_checker(
            "https://github.com/jaysonsantos/mambembe",
            Version::parse("1.2.0").unwrap(),
        )
        .unwrap();
        assert_eq!(github.checker_name(), "github");
    }
}
