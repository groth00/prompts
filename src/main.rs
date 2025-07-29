use std::fs;

use clap::{Parser, Subcommand};
use iced::window::{get_latest, maximize};

mod image_metadata;
use image_metadata::extract_image_metadata;

mod db;
mod files;
mod nai;
mod ui;

use db::import_from_dir;
use nai::{Character, ImageGenRequest, ImageShape, Requester};
use ui::{State, subscribe, update, view};

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
                .subscription(subscribe)
                .run_with(|| {
                    (
                        State::default(),
                        get_latest().and_then(|id| maximize(id, true)),
                    )
                })?;
        }
        Commands::Gen => {
            let s = fs::read_to_string("prompt.txt").expect("open prompt.txt");

            let mut req = ImageGenRequest::default();
            let mut base_prompt = "";
            let mut chars = vec![];
            req.seed(rand::random_range(1e9..9e9) as u64);

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

            runtime.block_on(async {
                let requester = Requester::default();
                if let Err(e) = requester.generate_image(ImageShape::Portrait, req).await {
                    eprintln!("{:?}", e);
                }
            });
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
    Dir { path: String },
}
