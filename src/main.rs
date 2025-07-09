use std::error::Error;

use clap::{Parser, Subcommand};

mod db;

mod nai;
use nai::ImageShape;

mod prompt;
use prompt::Character;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().expect("dotenv");

    let args = Args::parse();
    match &args.command {
        Commands::Gen { action } => {
            let base_prompt = "library";
            let char1_prompt = "1girl, standing, reading book, looking at book";
            let mut char1 = Character::default();
            char1.prompt = char1_prompt;
            let negative_prompt = "lowres";

            nai::generate_image(
                ImageShape::Portrait,
                base_prompt,
                &[char1],
                Some(negative_prompt),
            )
            .await?;
        }
        Commands::Todo { action } => {
            todo!();
        }
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
    Todo {
        #[command(subcommand)]
        action: TodoCmd,
    },
}

#[derive(Subcommand)]
enum GenerateCmd {
    Foo,
    Bar,
}

#[derive(Subcommand)]
enum TodoCmd {
    Todo,
}
