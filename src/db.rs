use rusqlite::{Connection, Error};

pub fn get_prompt(name: &'static str) -> Result<String, Error> {
    let conn = Connection::open("prompts.db")?;

    Ok(String::new())
}
