use clap::{Parser, Subcommand};
use iced::{
    Subscription,
    window::{get_latest, maximize},
};

mod image_metadata;
use image_metadata::extract_image_metadata;

mod db;
mod files;
mod nai;
mod ui;

use crate::{
    db::import_from_dir,
    ui::{State, event_subscribe, run_channel_subscription, update, view},
};

fn main() -> iced::Result {
    dotenvy::dotenv().expect("dotenv");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let args = Args::parse();

    match &args.command {
        Commands::Ui => {
            iced::application("NovelAI Desktop App", update, view)
                .subscription(|state| {
                    Subscription::batch([event_subscribe(state), run_channel_subscription()])
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
