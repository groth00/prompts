use std::{fs, sync::LazyLock};

use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use iced::{
    Subscription,
    window::{get_latest, maximize},
};

mod image_metadata;
use image_metadata::extract_image_metadata;

mod db;
mod files;
mod files2;
mod nai;
mod ui;

use crate::{
    db::import_from_dir,
    ui::{
        State, event_subscribe, run_channel_subscription, run_fsevent_subscription, update, view,
    },
};

static PROJECT_DIRS: LazyLock<ProjectDirs> = LazyLock::new(|| {
    let proj_dirs = ProjectDirs::from("com", "groth", "prompts")
        .expect("can't create project directories on this platform");

    let data_dir = proj_dirs.data_dir();
    match fs::exists(data_dir) {
        Ok(b) => {
            if !b {
                fs::create_dir_all(data_dir).expect("failed to create project data_dir");
            }
        }
        Err(e) => panic!("{}", e),
    };

    let output_dir = data_dir.join("output");
    match fs::exists(&output_dir) {
        Ok(b) => {
            if !b {
                fs::create_dir(&output_dir).expect("failed to create image output dir");
            }
        }
        Err(e) => panic!("{}", e),
    }

    proj_dirs
});

fn main() -> iced::Result {
    dotenvy::dotenv().expect("dotenv");

    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    match &args.command {
        Commands::Ui => {
            std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("cannot access data_dir");

            iced::application("NovelAI Prompts", update, view)
                .subscription(|state| {
                    Subscription::batch([
                        event_subscribe(state),
                        run_channel_subscription(),
                        // run_fsevent_subscription(),
                    ])
                })
                .theme(|state| state.selected_theme.clone())
                .run_with(|| {
                    (
                        State::default(),
                        get_latest().and_then(|id| maximize(id, true)),
                    )
                })?;
        }
        Commands::Metadata { path } => {
            let im = image::open(path).expect("open");
            if let Ok(map) = extract_image_metadata(im) {
                if let Ok(ser) = serde_json::to_string_pretty(&map) {
                    println!("{:?}", ser);
                }
            }
        }
        Commands::Import { action } => match action {
            ImportCmd::Dir { path } => runtime.block_on(async {
                match import_from_dir(path).await {
                    Ok(_) => eprintln!("import ok"),
                    Err(e) => eprintln!("import error: {:?}", e),
                }
            }),
        },
    }

    Ok(())
}

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Ui,
    Metadata {
        path: String,
    },
    Import {
        #[command(subcommand)]
        action: ImportCmd,
    },
}

#[derive(Subcommand)]
enum ImportCmd {
    Dir { path: String },
}
