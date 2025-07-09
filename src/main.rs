#![allow(dead_code)]
use std::error::Error;

use clap::{Parser, Subcommand};

mod db;

mod image_metadata;
use image_metadata::extract_image_metadata;

mod import;

mod nai;
use nai::ImageShape;

mod prompt;
use prompt::{Character, Position};
use rusqlite::Connection;

use crate::import::insert_characters_series;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().expect("dotenv");

    let args = Args::parse();
    match &args.command {
        Commands::Gen { action: _ } => {
            let base_prompt = "library";
            let char1 = Character::new()
                .prompt("1girl, standing, looking at book")
                .center(Position::R2C2)
                .finish();
            let negative_prompt = "lowres";

            nai::generate_image(
                ImageShape::Portrait,
                base_prompt,
                &[char1],
                Some(negative_prompt),
            )
            .await?;
        }
        Commands::Metadata { path } => {
            let map = extract_image_metadata(path)?;
            let pretty = serde_json::to_string_pretty(&map)?;
            println!("{}", pretty);
        }
        Commands::Import { action } => match action {
            ImportCmd::Danbooru => {
                let mut conn = Connection::open(std::env::var("SQLITE_URL").expect("SQLITE_URL"))?;

                let input = [
                    (
                        "text/artist_combos.txt",
                        include_str!("../sql/i_artist_combos.sql"),
                    ),
                    ("text/artists.txt", include_str!("../sql/i_artists.sql")),
                    ("text/tags.txt", include_str!("../sql/i_tags.sql")),
                ];

                for i in input {
                    let rows = import::insert_lines(&mut conn, i.0, i.1)?;
                    println!("added: {} tags", rows);
                }
                insert_characters_series(&mut conn, "text/characters.txt")?;
            }
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
    Gen {
        #[command(subcommand)]
        action: GenerateCmd,
    },
    Metadata {
        path: String,
    },
    Import {
        #[command(subcommand)]
        action: ImportCmd,
    },
}

#[derive(Subcommand)]
enum GenerateCmd {
    Foo,
    Bar,
}

#[derive(Subcommand)]
enum ImportCmd {
    Danbooru,
}
