use std::{fs, path::Path, time::UNIX_EPOCH};

use iced::widget::shader::wgpu::naga::FastHashMap;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::distr::{Alphanumeric, SampleString};
use rusqlite::{Connection, Error, OptionalExtension, params};

use crate::ui::get_prompt_metadata;

#[derive(Debug, Clone)]
pub struct SqliteError {
    pub err: String,
}

impl SqliteError {
    pub fn new(e: rusqlite::Error) -> Self {
        Self { err: e.to_string() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Template,
    Character,
    Base,
}

pub enum PromptData {
    Template(Template),
    CharacterPrompt(String),
    BasePrompt(String),
    Character(PromptDb),
    Base(PromptDb),
}

pub struct Template {
    pub base: String,
    pub characters: [Option<String>; 6],
}

struct TemplateWithName {
    name: String,
    base: String,
    characters: [Option<String>; 6],
}

pub struct PromptDb {
    pub name: String,
    pub prompt: String,
}

pub fn update_prompt_name(
    pool: Pool<SqliteConnectionManager>,
    table: PromptKind,
    old_name: String,
    new_name: String,
) -> Result<usize, Error> {
    let table_name = match table {
        PromptKind::Character => "characters",
        PromptKind::Base => "base",
        PromptKind::Template => "templates",
    };
    let update = format!("UPDATE {} SET name = ?1 WHERE name = ?2", table_name);

    let conn = pool.get().unwrap();
    conn.execute(&update, [&new_name, &old_name])
}

pub fn fetch_prompts(
    pool: Pool<SqliteConnectionManager>,
) -> Result<
    (
        Vec<String>,
        FastHashMap<String, String>,
        Vec<String>,
        FastHashMap<String, String>,
        Vec<String>,
        FastHashMap<String, Template>,
    ),
    Error,
> {
    let conn = pool.get().unwrap();
    let mut s_base = conn.prepare("SELECT name, t FROM base ORDER BY ts DESC")?;
    let mut s_char = conn.prepare("SELECT name, t FROM characters ORDER BY ts DESC")?;
    let mut s_templates = conn.prepare(include_str!("../sql/s_template_all.sql"))?;

    let mut base_options = Vec::new();
    let mut base_map = FastHashMap::default();

    let mut character_options = Vec::new();
    let mut character_map = FastHashMap::default();

    let mut template_options = Vec::new();
    let mut template_map = FastHashMap::default();

    let b = s_base.query_map([], |r| {
        Ok(PromptDb {
            name: r.get::<usize, String>(0)?,
            prompt: r.get::<usize, String>(1)?,
        })
    })?;
    for row in b {
        let row = row?;
        base_options.push(row.name.clone());
        base_map.insert(row.name, row.prompt);
    }

    let c = s_char.query_map([], |r| {
        Ok(PromptDb {
            name: r.get::<usize, String>(0)?,
            prompt: r.get::<usize, String>(1)?,
        })
    })?;
    for row in c {
        let row = row?;
        character_options.push(row.name.clone());
        character_map.insert(row.name, row.prompt);
    }

    let t = s_templates.query_map([], |r| {
        Ok(TemplateWithName {
            name: r.get::<usize, String>(0)?,
            base: r.get::<usize, String>(1)?,
            characters: [
                r.get::<usize, Option<String>>(2)?,
                r.get::<usize, Option<String>>(3)?,
                r.get::<usize, Option<String>>(4)?,
                r.get::<usize, Option<String>>(5)?,
                r.get::<usize, Option<String>>(6)?,
                r.get::<usize, Option<String>>(7)?,
            ],
        })
    })?;
    for row in t {
        let row = row?;
        template_options.push(row.name.clone());
        template_map.insert(
            row.name,
            Template {
                base: row.base,
                characters: row.characters,
            },
        );
    }

    Ok((
        base_options,
        base_map,
        character_options,
        character_map,
        template_options,
        template_map,
    ))
}

pub fn load_prompt(
    conn: &mut Connection,
    kind: PromptKind,
    name: String,
) -> Result<PromptData, Error> {
    let s_template = include_str!("../sql/s_template.sql");
    let s_char = "SELECT t FROM characters WHERE name = ?1";
    let s_base = "SELECT t FROM base WHERE name = ?1";

    let ret = match kind {
        PromptKind::Base => PromptData::BasePrompt(
            conn.query_one(s_base, params![name], |r| r.get::<usize, String>(0))?,
        ),
        PromptKind::Character => {
            PromptData::CharacterPrompt(
                conn.query_one(s_char, params![name], |r| r.get::<usize, String>(0))?,
            )
        }
        PromptKind::Template => {
            PromptData::Template(conn.query_one(s_template, params![name], |r| {
                Ok(Template {
                    base: r.get::<usize, String>(0)?,
                    characters: [
                        r.get::<usize, Option<String>>(1)?,
                        r.get::<usize, Option<String>>(2)?,
                        r.get::<usize, Option<String>>(3)?,
                        r.get::<usize, Option<String>>(4)?,
                        r.get::<usize, Option<String>>(5)?,
                        r.get::<usize, Option<String>>(6)?,
                    ],
                })
            })?)
        }
    };

    Ok(ret)
}

pub async fn save_prompt(
    pool: Pool<SqliteConnectionManager>,
    metadata: Vec<(i64, String, Vec<String>)>,
) -> Result<(), SqliteError> {
    let mut conn = pool.get().unwrap();
    let tx = conn.transaction().map_err(|e| SqliteError::new(e))?;
    let insert_base = include_str!("../sql/i_base.sql");
    let insert_char = include_str!("../sql/i_char.sql");
    let insert_template = include_str!("../sql/i_template.sql");
    let select_base = "SELECT id FROM base WHERE t = ?1";
    let select_char = "SELECT id FROM characters WHERE t = ?1";

    let mut rng = rand::rng();

    {
        let mut base = tx.prepare(insert_base).map_err(|e| SqliteError::new(e))?;
        let mut char = tx.prepare(insert_char).map_err(|e| SqliteError::new(e))?;
        let mut template = tx
            .prepare(insert_template)
            .map_err(|e| SqliteError::new(e))?;
        let mut select_base = tx.prepare(select_base).map_err(|e| SqliteError::new(e))?;
        let mut select_char = tx.prepare(select_char).map_err(|e| SqliteError::new(e))?;

        for (ts, prompt, characters) in metadata {
            let name = Alphanumeric.sample_string(&mut rng, 8);

            let b = match base
                .query_row(params![ts, name, prompt], |r| r.get::<usize, i64>(0))
                .optional()
            {
                Ok(Some(id)) => Ok(id),
                Ok(None) => Ok(select_base
                    .query_row(params![prompt], |r| r.get::<usize, i64>(0))
                    .map_err(|e| SqliteError::new(e)))?,
                Err(e) => Err(SqliteError::new(e)),
            }?;

            let mut c: Vec<Option<i64>> = characters
                .iter()
                .try_fold(Vec::new(), |mut acc, s| {
                    let name = Alphanumeric.sample_string(&mut rng, 8);

                    match char
                        .query_row(params![ts, name, s], |r| r.get::<usize, i64>(0))
                        .optional()?
                    {
                        Some(id) => acc.push(Some(id)),
                        None => acc.push(Some(
                            select_char.query_row(params![s], |r| r.get::<usize, i64>(0))?,
                        )),
                    }
                    Ok::<Vec<Option<i64>>, Error>(acc)
                })
                .map_err(|e| SqliteError::new(e))?;

            let mut count = 6 - c.len();
            while count > 0 {
                c.push(None);
                count -= 1;
            }

            template
                .execute(params![ts, name, b, c[0], c[1], c[2], c[3], c[4], c[5]])
                .map_err(|e| SqliteError::new(e))?;
            eprintln!("inserted {}", name);
        }
    }

    tx.commit().map_err(|e| SqliteError::new(e))?;

    Ok(())
}

pub async fn update_prompt(
    pool: Pool<SqliteConnectionManager>,
    kind: PromptKind,
    name: String,
    content: String,
) -> Result<(), SqliteError> {
    let conn = pool.get().unwrap();
    let query = match kind {
        PromptKind::Base => "UPDATE base SET t = ?1 WHERE name = ?2",
        PromptKind::Character => "UPDATE characters SET t = ?1 WHERE name = ?2",
        _ => unreachable!(),
    };
    match conn.execute(query, &[&content, &name]) {
        Ok(_rows_changed) => Ok(()),
        Err(e) => Err(SqliteError::new(e)),
    }
}

pub async fn delete_prompt(
    pool: Pool<SqliteConnectionManager>,
    kind: PromptKind,
    name: String,
) -> Result<(), SqliteError> {
    let conn = pool.get().unwrap();
    let query = match kind {
        PromptKind::Base => "DELETE FROM base WHERE name = ?1",
        PromptKind::Character => "DELETE FROM characters WHERE name = ?1",
        PromptKind::Template => "DELETE FROM templates WHERE name = ?1",
    };
    match conn.execute(query, &[&name]) {
        Ok(_rows_changed) => Ok(()),
        Err(e) => Err(SqliteError::new(e)),
    }
}

pub async fn import_from_dir<P: AsRef<Path>>(dir: P) -> Result<usize, SqliteError> {
    let mut metadata: Vec<(i64, String, Vec<String>)> = vec![];
    let len = metadata.len();
    let mut read_dir = fs::read_dir(dir).expect("read_dir");
    while let Some(Ok(entry)) = read_dir.next() {
        if let Ok(meta) = entry.metadata() {
            let ts = meta
                .modified()
                .expect("modified")
                .duration_since(UNIX_EPOCH)
                .expect("duration_since")
                .as_secs() as i64;
            if let Some((_seed, prompt, characters)) = get_prompt_metadata(entry.path()) {
                metadata.push((ts, prompt, characters));
            }
        }
    }

    let manager = SqliteConnectionManager::file(std::env::var("SQLITE_URL").unwrap());
    let pool = r2d2::Pool::new(manager).unwrap();

    save_prompt(pool, metadata).await?;

    Ok(len)
}
