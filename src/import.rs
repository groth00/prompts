use std::fs;

use rusqlite::{Connection, Error, config::DbConfig};

pub fn import() -> Result<(), Error> {
    let mut conn = Connection::open(std::env::var("SQLITE_URL").expect("SQLITE_URL"))?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)?;

    let mut rows = insert_tags(&mut conn, "text/tags.txt")?;
    println!("added {} from {}", rows, "tags");
    rows = insert_characters_series(&mut conn, "text/characters.txt")?;
    println!("added {} from {}", rows, "characters");

    let inputs = [
        (
            "text/artist_combos.txt",
            include_str!("../sql/i_artist_combos.sql"),
        ),
        ("text/artists.txt", include_str!("../sql/i_artists.sql")),
    ];
    for i in inputs {
        rows = insert_single(&mut conn, i.0, i.1)?;
        println!("added {} from {}", rows, i.0);
    }

    let inputs = [
        (
            "text/prompt_starts.txt",
            include_str!("../sql/i_prompt_starts.sql"),
        ),
        ("text/locations.txt", include_str!("../sql/i_locations.sql")),
        ("text/scenes.txt", include_str!("../sql/i_scenes.sql")),
        ("text/quality.txt", include_str!("../sql/i_quality.sql")),
        ("text/negatives.txt", include_str!("../sql/i_negatives.sql")),
        (
            "text/characters_desc.txt",
            include_str!("../sql/i_characters_desc.sql"),
        ),
        ("text/outfits.txt", include_str!("../sql/i_outfits.sql")),
        ("text/postures.txt", include_str!("../sql/i_postures.sql")),
        ("text/actions.txt", include_str!("../sql/i_actions.sql")),
        ("text/body.txt", include_str!("../sql/i_body.sql")),
        (
            "text/expressions.txt",
            include_str!("../sql/i_expressions.sql"),
        ),
    ];
    for i in inputs {
        let table_name = i.0.split("/").last().unwrap().split(".").next().unwrap();
        rows = insert_double(&mut conn, i.0, i.1, table_name)?;
        println!("added {} from {}", rows, i.0);
    }

    Ok(())
}

pub fn insert_single(
    conn: &mut Connection,
    path: &'static str,
    sql: &'static str,
) -> Result<i64, Error> {
    let text = fs::read_to_string(&path).expect("open file");
    let name = path.split("/").last().unwrap().split(".").next().unwrap();

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(sql)?;
        for line in text.lines() {
            if line.is_empty() || line.starts_with("*") {
                continue;
            }
            stmt.execute([line])?;
        }
    }
    tx.commit()?;

    let q = format!("SELECT COUNT(1) FROM {}", name);
    let rows = conn.query_one(&q, [], |r| r.get::<usize, i64>(0))?;
    Ok(rows)
}

pub fn insert_double(
    conn: &mut Connection,
    path: &'static str,
    sql: &'static str,
    table: &'static str,
) -> Result<i64, Error> {
    let text = fs::read_to_string(&path).expect("open file");

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(sql)?;
        for line in text.lines() {
            let parts = line.split_once("|").unwrap();
            stmt.execute([parts.0, parts.1])?;
        }
    }
    tx.commit()?;

    let q = format!("SELECT COUNT(1) FROM {}", table);
    let rows = conn.query_one(&q, [], |r| r.get::<usize, i64>(0))?;
    Ok(rows)
}

pub fn insert_tags(conn: &mut Connection, path: &'static str) -> Result<i64, Error> {
    let text = fs::read_to_string(&path).expect("open file");

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(include_str!("../sql/i_tags.sql"))?;

        let mut category = "";

        for line in text.lines() {
            if line.starts_with("*") {
                category = line.trim_start_matches("* ");
            } else if !line.is_empty() {
                stmt.execute([line, category])?;
            }
        }
    }
    tx.commit()?;

    let q = "SELECT COUNT(1) FROM tags";
    let rows = conn.query_one(&q, [], |r| r.get::<usize, i64>(0))?;
    Ok(rows)
}

pub fn insert_characters_series(conn: &mut Connection, path: &'static str) -> Result<i64, Error> {
    let text = fs::read_to_string(&path).expect("open file");

    let tx = conn.transaction()?;
    {
        let mut series = tx.prepare(include_str!("../sql/i_series.sql"))?;
        let mut chars = tx.prepare(include_str!("../sql/i_characters.sql"))?;

        let mut id = 0;

        for line in text.lines() {
            if line.starts_with("*") {
                let name = line.trim_start_matches("* ");
                id = series.query_one([name], |r| r.get::<usize, i64>(0))?;
            } else if !line.is_empty() {
                chars.execute([line, &id.to_string()])?;
            }
        }
    }
    tx.commit()?;

    let q = "SELECT COUNT(1) FROM characters";
    let rows = conn.query_one(&q, [], |r| r.get::<usize, i64>(0))?;
    Ok(rows)
}
