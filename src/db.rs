use std::{fs, path::Path, time::UNIX_EPOCH};

use iced::widget::shader::wgpu::naga::FastHashMap;
use rand::distr::{Alphanumeric, SampleString};
use rusqlite::{Connection, Error, OptionalExtension, params};

use crate::ui::get_prompt_metadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Template,
    CharacterPrompt,
    BasePrompt,
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
    pub c1: Option<String>,
    pub c2: Option<String>,
    pub c3: Option<String>,
    pub c4: Option<String>,
    pub c5: Option<String>,
    pub c6: Option<String>,
}

struct TemplateWithName {
    name: String,
    base: String,
    c1: Option<String>,
    c2: Option<String>,
    c3: Option<String>,
    c4: Option<String>,
    c5: Option<String>,
    c6: Option<String>,
}

pub struct PromptDb {
    pub name: String,
    pub prompt: String,
}

pub fn fetch_prompts(
    conn: &mut Connection,
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
            c1: r.get::<usize, Option<String>>(2)?,
            c2: r.get::<usize, Option<String>>(3)?,
            c3: r.get::<usize, Option<String>>(4)?,
            c4: r.get::<usize, Option<String>>(5)?,
            c5: r.get::<usize, Option<String>>(6)?,
            c6: r.get::<usize, Option<String>>(7)?,
        })
    })?;
    for row in t {
        let row = row?;
        template_options.push(row.name.clone());
        template_map.insert(
            row.name,
            Template {
                base: row.base,
                c1: row.c1,
                c2: row.c2,
                c3: row.c3,
                c4: row.c4,
                c5: row.c5,
                c6: row.c6,
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
        PromptKind::BasePrompt => {
            PromptData::BasePrompt(
                conn.query_one(s_base, params![name], |r| r.get::<usize, String>(0))?,
            )
        }
        PromptKind::CharacterPrompt => {
            PromptData::CharacterPrompt(
                conn.query_one(s_char, params![name], |r| r.get::<usize, String>(0))?,
            )
        }
        PromptKind::Template => {
            PromptData::Template(conn.query_one(s_template, params![name], |r| {
                Ok(Template {
                    base: r.get::<usize, String>(0)?,
                    c1: r.get::<usize, Option<String>>(1)?,
                    c2: r.get::<usize, Option<String>>(2)?,
                    c3: r.get::<usize, Option<String>>(3)?,
                    c4: r.get::<usize, Option<String>>(4)?,
                    c5: r.get::<usize, Option<String>>(5)?,
                    c6: r.get::<usize, Option<String>>(6)?,
                })
            })?)
        }
        _ => unreachable!(),
    };

    Ok(ret)
}

pub fn save_prompt(
    conn: &mut Connection,
    metadata: &Vec<(i64, String, Vec<String>)>,
) -> Result<(), Error> {
    let tx = conn.transaction()?;
    let insert_base = include_str!("../sql/i_base.sql");
    let insert_char = include_str!("../sql/i_char.sql");
    let insert_template = include_str!("../sql/i_template.sql");
    let select_base = "SELECT id FROM base WHERE t = ?1";
    let select_char = "SELECT id FROM characters WHERE t = ?1";

    let mut rng = rand::rng();

    {
        let mut base = tx.prepare(insert_base)?;
        let mut char = tx.prepare(insert_char)?;
        let mut template = tx.prepare(insert_template)?;
        let mut select_base = tx.prepare(select_base)?;
        let mut select_char = tx.prepare(select_char)?;

        for (ts, prompt, characters) in metadata {
            let name = Alphanumeric.sample_string(&mut rng, 8);

            let b = match base
                .query_row(params![ts, name, prompt], |r| r.get::<usize, i64>(0))
                .optional()
            {
                Ok(Some(id)) => Ok(id),
                Ok(None) => Ok(select_base.query_row(params![prompt], |r| r.get::<usize, i64>(0))?),
                Err(e) => Err(e),
            }?;

            let mut c: Vec<Option<i64>> =
                characters.iter().try_fold(Vec::new(), |mut acc, s| {
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
                })?;

            let mut count = 6 - c.len();
            while count > 0 {
                c.push(None);
                count -= 1;
            }

            template.execute(params![ts, name, b, c[0], c[1], c[2], c[3], c[4], c[5]])?;
            println!("inserted {}", name);
        }
    }

    tx.commit()?;

    Ok(())
}

pub fn import_from_dir<P: AsRef<Path>>(dir: P) -> Result<usize, Error> {
    let mut metadata: Vec<(i64, String, Vec<String>)> = vec![];
    let mut read_dir = fs::read_dir(dir).expect("read_dir");
    while let Some(Ok(entry)) = read_dir.next() {
        if let Ok(meta) = entry.metadata() {
            let ts = meta
                .modified()
                .expect("modified")
                .duration_since(UNIX_EPOCH)
                .expect("duration_since")
                .as_secs() as i64;
            if let Some((prompt, characters)) = get_prompt_metadata(entry.path()) {
                metadata.push((ts, prompt, characters));
            }
        }
    }

    let mut conn = Connection::open(std::env::var("SQLITE_URL").expect("set SQLITE_URL in .env"))?;

    save_prompt(&mut conn, &metadata)?;

    Ok(metadata.len())
}
