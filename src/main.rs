use std::{error::Error, fs};

use clap::{Parser, Subcommand};
use iced::window::{get_latest, maximize};

mod image_metadata;
use image_metadata::extract_image_metadata;

mod db;
mod files;
mod import;
mod nai;
mod prompt;
mod ui;

use import::import;
use nai::ImageShape;
use prompt::Character;
use ui::{State, subscribe, update, view};

use crate::nai::{ImageGenRequest, Requester};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().expect("dotenv");

    let args = Args::parse();
    match &args.command {
        Commands::Ui => iced::application("Pane Example", update, view)
            .subscription(subscribe)
            .run_with(|| {
                (
                    State::default(),
                    get_latest().and_then(|id| maximize(id, true)),
                )
            })?,
        Commands::Gen => {
            let s = fs::read_to_string("prompt.txt")?;

            let mut req = ImageGenRequest::default();
            let mut base_prompt = "";
            let mut chars = vec![];

            for (i, s) in s.lines().enumerate() {
                if i == 0 {
                    base_prompt = s;
                } else {
                    chars.push(Character::new().prompt(s.into()).finish());
                }
            }
            req.prompt(base_prompt.to_owned());

            for ch in chars {
                req.add_character(&ch);
            }

            let requester = Requester::default();
            if let Err(e) = requester.generate_image(ImageShape::Portrait, req).await {
                eprintln!("{:?}", e);
            }
        }
        Commands::Metadata { path } => {
            let map = extract_image_metadata(path)?;
            let pretty = serde_json::to_string_pretty(&map)?;
            println!("{}", pretty);
        }
        Commands::Import { action } => match action {
            ImportCmd::Danbooru => import()?,
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
    Gen,
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
    Danbooru,
}
