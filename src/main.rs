mod commands;
mod package;
mod version_checker;

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::WrapErr, Result};
use commands::process_package;
use directories::ProjectDirs;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use structopt::{clap::arg_enum, StructOpt};
use tokio::fs;
use tracing_error::ErrorLayer;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::commands::{list_user_packages, process_user};

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
enum Arguments {
    ProcessPackage {
        #[structopt(short, long)]
        package_name: String,
    },
    ProcessUser {
        #[structopt(short, long)]
        username: String,
    },
    ListUserPackages {
        #[structopt(short, long)]
        username: String,
        #[structopt(short, long)]
        output_type: OutputType,
    },
}

arg_enum! {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum OutputType {
        Json,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_error_handlers()?;
    let args = Arguments::from_args();
    write_helper_script().await?;

    match args {
        Arguments::ProcessPackage { package_name } => process_package(&package_name).await?,
        Arguments::ProcessUser { username } => process_user(&username).await?,
        Arguments::ListUserPackages {
            username,
            output_type: _,
        } => {
            let packages = list_user_packages(&username)
                .await
                .wrap_err("failed to list user's packages")?;
            let packages: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
            let json_output = serde_json::to_string(&packages)?;
            println!("{}", json_output);
        }
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
