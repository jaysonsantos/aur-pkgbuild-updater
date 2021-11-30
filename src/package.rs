use camino::Utf8PathBuf;
use color_eyre::{
    eyre::{eyre, WrapErr},
    Result, Section, SectionExt,
};
use futures::StreamExt;
use lazy_static::lazy_static;
use scraper::Selector;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::{fs, process::Command};
use tracing::{info, instrument, trace};

use crate::version::LenientVersion;
use crate::{version_checker::get_version_checker, CACHE_DIR, CLIENT, HELPER_SCRIPT};

pub const VERSION_PLACEHOLDER: &str = "_VERSION_PLACEHOLDER_";

lazy_static! {
    static ref PROJECTS_SELECTOR: Selector = Selector::parse(".results td:nth-child(1) a").unwrap();
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Package {
    pub name: String,
    repository: String,
    clone_directory: Utf8PathBuf,
    current_version: Option<LenientVersion>,
    current_download_url: Option<String>,
    current_sha2_digest: Option<String>,
}

impl Package {
    pub fn new(name: &str) -> Self {
        Self::new_with_default_repository(name)
    }

    fn new_with_default_repository(name: &str) -> Self {
        Self::new_with_custom_repository(name, format!("aur.archlinux.org:{}.git", name))
    }

    fn new_with_custom_repository(name: &str, repository: String) -> Self {
        Self {
            name: name.to_string(),
            repository,
            clone_directory: CACHE_DIR.join(name),
            current_version: None,
            current_download_url: None,
            current_sha2_digest: None,
        }
    }

    #[instrument(skip(self), fields(name = self.name.as_str()))]
    pub async fn process(&mut self) -> Result<()> {
        info!("Processing");
        self.clone_repository().await?;
        self.cleanup().await?;
        self.parse_pkgbuild().await?;
        if let Some(new_version) = self.update().await? {
            self.commit(&new_version).await?;
            self.push().await?;
        }

        Ok(())
    }

    pub async fn parse_packages(url: &str) -> Result<Vec<Package>> {
        let response = CLIENT
            .get(url)
            .send()
            .await
            .wrap_err("failed to load user's page")?;

        let body = response
            .text()
            .await
            .wrap_err("failed to get the content of user's page")?;

        Ok(Self::parse_packages_from_html(&body))
    }

    fn parse_packages_from_html(body: &str) -> Vec<Package> {
        let users_page = scraper::Html::parse_document(body);
        let projects: Vec<String> = users_page
            .select(&PROJECTS_SELECTOR)
            .map(|a| a.text().collect())
            .collect();

        projects
            .iter()
            .map(|project| Package::new(project))
            .collect()
    }

    #[instrument(skip(self))]
    async fn clone_repository(&self) -> Result<()> {
        if self.clone_directory.exists() {
            return Ok(());
        }
        let status = Command::new("git")
            .arg("clone")
            .arg("-v")
            .arg(&self.repository)
            .arg(&self.clone_directory)
            .status()
            .await
            .wrap_err("failed to clone repository")?;

        if !status.success() {
            return Err(eyre!("failed to clone repository"));
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup(&self) -> Result<()> {
        let status = Command::new("git")
            .args(&["remote", "update", "-p"])
            .current_dir(&self.clone_directory)
            .status()
            .await
            .wrap_err("failed to update repository")?;

        if !status.success() {
            return Err(eyre!(
                "failed to update repository on {:?}",
                &self.clone_directory
            ));
        }

        let status = Command::new("git")
            .args(&["reset", "--hard", "origin/master"])
            .current_dir(&self.clone_directory)
            .status()
            .await
            .wrap_err("failed to reset to master")?;

        if !status.success() {
            return Err(eyre!("failed to reset to master"));
        }

        Ok(())
    }

    #[instrument(skip(self), fields(name = self.name.as_str()))]
    async fn parse_pkgbuild(&mut self) -> Result<()> {
        let response = Command::new("bash")
            .arg(&*HELPER_SCRIPT)
            .arg(&self.pkg_build_file())
            .output()
            .await?;

        let helper_output = String::from_utf8_lossy(&response.stdout);
        if !response.status.success() {
            let stderr = String::from_utf8_lossy(&response.stderr);
            return Err(eyre!("failed to run helper script")
                .section(helper_output.to_string().header("Stdout"))
                .section(stderr.to_string().header("Stderr")));
        }
        for line in helper_output.lines() {
            let mut components = line.split('=');
            let variable = components
                .next()
                .map(|v| v.trim())
                .ok_or_else(|| eyre!("help did not return a valid variable"))?;
            let value = components
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("=")
                .trim()
                .to_string();
            match variable {
                "pkgver" => {
                    let parsed_version = LenientVersion::parse(&value)
                        .map_err(|e| eyre!("failed to parse version with {:?} {:?}", value, e))?;
                    self.current_version = Some(parsed_version)
                }
                "source" => self.current_download_url = Some(value),
                "sha256sums" => self.current_sha2_digest = Some(value),
                v => panic!("unsupported variable {:?}", v),
            }
        }

        if self.current_version.is_none() {
            return Err(eyre!(
                "helper script could not determine the current version"
            ));
        }

        if self.current_download_url.is_none() {
            return Err(eyre!(
                "helper script could not determine the current download url"
            ));
        }

        Ok(())
    }

    fn pkg_build_file(&self) -> Utf8PathBuf {
        self.clone_directory.join("PKGBUILD")
    }

    fn src_info_file(&self) -> Utf8PathBuf {
        self.clone_directory.join(".SRCINFO")
    }

    #[instrument(skip(self), fields(name = % self.name))]
    async fn update(&self) -> Result<Option<String>> {
        let current_download_url = self.current_download_url.as_ref().unwrap();
        let mut version_checker = get_version_checker(
            current_download_url,
            self.current_version.as_ref().unwrap().clone(),
        )
        .wrap_err("failed to get a version checker")?;
        version_checker
            .fetch_last_version(&self.get_file_template()?)
            .await?;
        if !version_checker.has_newer_version() {
            info!("already on the latest version");
            return Ok(None);
        }

        let pkg_build_file = self.pkg_build_file();
        let contents = fs::read_to_string(&pkg_build_file).await?;
        let current_version = self.current_version.as_ref().unwrap();
        let remote_version = version_checker.get_remote_version().unwrap();
        let current_hash = self.current_sha2_digest.as_ref().unwrap().as_str();
        let remote_hash = calculate_hash(version_checker.get_download_url().unwrap()).await?;
        info!(message = "Updating version", %current_version, %current_hash, %remote_version, %remote_hash);
        let clean = remote_version.clean_original_value();
        let contents = contents
            .replace(&current_version.original_value(), clean)
            .replace(&current_hash, &remote_hash);

        trace!(message = "Final PKGBUILD file", %contents);
        fs::write(&pkg_build_file, contents).await?;

        let response = Command::new("makepkg")
            .args(&["--clean", "--force", "--syncdeps", "--noconfirm"])
            .env("PACMAN", "yay") // Use yay so it can handle AUR dependencies
            .env("PACMAN_AUTH", "nice") // Small hack so makepkg doesn't try to use sudo
            .current_dir(&self.clone_directory)
            .spawn()?
            .wait()
            .await?;

        if !response.success() {
            return Err(eyre!("failed to make a package for the current version"));
        }

        self.write_src_info().await?;
        Ok(Some(remote_version.to_string()))
    }

    fn get_file_template(&self) -> Result<String> {
        let current_download_url = self.current_download_url.as_ref().unwrap();
        let parsed_url = url::Url::parse(current_download_url.as_str())?;
        let segments = parsed_url
            .path_segments()
            .ok_or_else(|| eyre!("Could not determine the download file"))?;
        let file_name = segments
            .last()
            .ok_or_else(|| eyre!("Could not determine the download file"))?;

        Ok(file_name.replace(
            self.current_version.as_ref().unwrap().to_string().as_str(),
            VERSION_PLACEHOLDER,
        ))
    }

    #[instrument(skip(self))]
    async fn write_src_info(&self) -> Result<()> {
        let response = Command::new("bash")
            .arg("-exc")
            .arg(&format!(
                "makepkg --printsrcinfo > {}",
                self.src_info_file()
            ))
            .current_dir(&self.clone_directory)
            .spawn()?
            .wait()
            .await?;

        if !response.success() {
            return Err(eyre!("failed to update .SRCINFO"));
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn commit(&self, version: &str) -> Result<()> {
        let response = Command::new("git")
            .args(&["commit", "-am"])
            .arg(&format!("Update to version {}", version))
            .current_dir(&self.clone_directory)
            .spawn()?
            .wait()
            .await?;
        if !response.success() {
            return Err(eyre!("failed to commit"));
        }
        Ok(())
    }

    #[instrument(skip(self))]
    async fn push(&self) -> Result<()> {
        let response = Command::new("git")
            .arg("push")
            .current_dir(&self.clone_directory)
            .spawn()?
            .wait()
            .await?;
        if !response.success() {
            return Err(eyre!("failed to push"));
        }
        Ok(())
    }
}

#[instrument]
async fn calculate_hash(url: &str) -> Result<String> {
    info!("Calculating hash for the downloaded URL");
    let mut hash = Sha256::default();
    let mut stream = CLIENT.get(url).send().await?.bytes_stream();
    while let Some(chunk) = stream.next().await {
        hash.update(chunk?);
    }

    let final_hash = format!("{:x}", hash.finalize());
    info!(message = "Done", %final_hash);
    Ok(final_hash)
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;
    use tokio::{fs, process::Command};
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::{setup_error_handlers, write_helper_script};

    use super::{calculate_hash, Package, CACHE_DIR};

    const TEST_PACKAGE: &[u8] = include_bytes!("../tests/fixtures/test-package.tar.gz");

    #[tokio::test]
    async fn test_parse_user_packages() {
        let fixture = fs::read_to_string("tests/fixtures/user-page.html")
            .await
            .unwrap();
        let packages = Package::parse_packages_from_html(&fixture);
        assert_eq!(packages.len(), 6);
        assert_eq!(packages[0].name, "balde");
        assert_eq!(packages[0].repository, "aur.archlinux.org:balde.git");
    }

    async fn setup_test_repository() -> TempDir {
        let remote_repository = TempDir::new("aur-autoupdater").unwrap();
        let response = Command::new("git")
            .args(&["init", "--bare"])
            .arg(remote_repository.as_ref())
            .status()
            .await
            .unwrap();
        assert!(response.success());

        let stub_repository = TempDir::new("aur-autoupdater-stub").unwrap();
        let response = Command::new("bash")
            .arg("tests/fixtures/setup-stub-repository.sh")
            .arg(stub_repository.as_ref())
            .arg(remote_repository.as_ref())
            .spawn()
            .unwrap()
            .wait()
            .await
            .unwrap();
        assert!(response.success());

        remote_repository
    }

    #[tokio::test]
    async fn test_package_process() {
        setup_error_handlers().ok();
        write_helper_script().await.unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:43987").unwrap();
        listener
            .local_addr()
            .expect("Failed to get server address.");

        let mock_server = MockServer::builder().listener(listener).start().await;
        Mock::given(method("GET"))
            .and(path("/0.1.1/test-package-0.1.1.tar.gz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(TEST_PACKAGE.to_vec(), "application/gzip"),
            )
            .mount(&mock_server)
            .await;
        fs::remove_dir_all(CACHE_DIR.join("test-package"))
            .await
            .ok();
        let repository = setup_test_repository().await;
        let mut package = Package::new_with_custom_repository(
            "test-package",
            repository.as_ref().to_string_lossy().to_string(),
        );

        package.process().await.unwrap();
    }

    #[tokio::test]
    async fn test_calculate_hash() {
        let expected_hash = "1d38233b764e0ac9f326cbd06474b6454cf80ccb3c6d0b44b7697a8f51e5891e";

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/test-package.tar.gz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(TEST_PACKAGE.to_vec(), "application/gzip"),
            )
            .mount(&mock_server)
            .await;

        assert_eq!(
            expected_hash,
            calculate_hash(&format!("{}/test-package.tar.gz", mock_server.uri()))
                .await
                .unwrap()
        )
    }
}
