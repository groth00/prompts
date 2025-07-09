use std::fs;

use rusqlite::{Connection, Error};

pub fn insert_lines(
    conn: &mut Connection,
    path: &'static str,
    sql: &'static str,
) -> Result<i64, Error> {
    let tags = fs::read_to_string(&path).expect("open file");
    let name = path.split("/").last().unwrap().split(".").next().unwrap();

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(sql)?;
        for line in tags.lines() {
            if !line.is_empty() {
                stmt.execute([line])?;
            }
        }
    }
    tx.commit()?;

    let q = format!("SELECT COUNT(1) FROM {}", name);
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
