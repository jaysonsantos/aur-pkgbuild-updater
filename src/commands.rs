use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use tracing::{error, instrument};

use crate::{package::Package, URL};

#[instrument]
pub async fn process_user(username: &str) -> Result<()> {
    let mut should_exit_with_failure = false;

    let mut packages = list_user_packages(username).await?;
    for package in packages.iter_mut() {
        if let Err(e) = package.process().await {
            error!(
                message = "Skipping package because of an error",
                ?package,
                error = ?e
            );
            should_exit_with_failure = true;
        }
    }

    if should_exit_with_failure {
        return Err(eyre!("Failed to process all packages"));
    }

    Ok(())
}

#[instrument]
pub async fn list_user_packages(username: &str) -> Result<Vec<Package>> {
    Package::parse_packages(&URL.replace("username", username)).await
}

#[instrument]
pub async fn process_package(package_name: &str) -> Result<()> {
    let mut package = Package::new(package_name);
    package
        .process()
        .await
        .wrap_err("failed to process package")
}
