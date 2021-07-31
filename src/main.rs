mod package;
mod version_checker;

use std::{io, process::exit};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::WrapErr, Result};
use directories::ProjectDirs;
use lazy_static::lazy_static;
use reqwest::Client;
use structopt::StructOpt;
use tokio::fs;
use tracing::error;
use tracing_error::ErrorLayer;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::package::Package;

const URL: &str = "https://aur.archlinux.org/packages/?K=username&SeB=m";

lazy_static! {
    pub static ref CLIENT: Client = reqwest::ClientBuilder::new()
        .user_agent(format!(
            "AUR-AutoUpdater (+{})",
            env!("CARGO_PKG_REPOSITORY")
        ))
        .build()
        .expect("failed to build request client");
    static ref PROJECT_DIR: ProjectDirs =
        ProjectDirs::from("com", "Jayson Reis", env!("CARGO_PKG_NAME"))
            .expect("failed to determine project directory");
    pub static ref CACHE_DIR: Utf8PathBuf = {
        let dir = Utf8Path::from_path(PROJECT_DIR.cache_dir())
            .unwrap()
            .to_path_buf();
        if !dir.exists() {
            ::std::fs::create_dir_all(&dir).expect("failed to create cache directory");
        }
        dir
    };
    pub static ref HELPER_SCRIPT: Utf8PathBuf = CACHE_DIR.join("helper.sh");
}

#[derive(Debug, StructOpt)]
struct Arguments {
    #[structopt(short, long)]
    username: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_error_handlers()?;
    let args = Arguments::from_args();
    let cache_dir = &*CACHE_DIR;
    match fs::create_dir(cache_dir).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        e => return e.wrap_err("failed to create cache_dir"),
    }

    write_helper_script().await?;
    let mut should_exit_with_failure = false;

    let mut packages = Package::parse_packages(&URL.replace("username", &args.username)).await?;
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
        exit(1);
    }

    Ok(())
}

pub(crate) fn setup_error_handlers() -> Result<()> {
    if tracing::dispatcher::has_been_set() {
        return Ok(());
    }
    let error_layer = ErrorLayer::default();
    let filter_layer = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?;
    let fmt_layer = fmt::layer().with_target(false);

    tracing_subscriber::Registry::default()
        .with(error_layer)
        .with(filter_layer)
        .with(fmt_layer)
        .try_init()?;

    color_eyre::install()?;
    Ok(())
}

async fn write_helper_script() -> Result<()> {
    fs::write(&*HELPER_SCRIPT, include_bytes!("helper.sh")).await?;
    Ok(())
}
