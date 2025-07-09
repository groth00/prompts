-- Add up migration script here
CREATE TABLE IF NOT EXISTS prompts(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  base_prompt INTEGER NOT NULL,
  num_characters INTEGER NOT NULL DEFAULT 1,
  ch_prompt1 INTEGER NOT NULL,
  ch_prompt2 INTEGER,
  ch_prompt3 INTEGER,
  ch_prompt4 INTEGER,
  ch_prompt5 INTEGER,
  ch_prompt6 INTEGER,
  FOREIGN KEY(base_prompt) REFERENCES base_prompts(id),
  FOREIGN KEY(ch_prompt1) REFERENCES characters(id),
  FOREIGN KEY(ch_prompt2) REFERENCES characters(id),
  FOREIGN KEY(ch_prompt3) REFERENCES characters(id),
  FOREIGN KEY(ch_prompt4) REFERENCES characters(id),
  FOREIGN KEY(ch_prompt5) REFERENCES characters(id),
  FOREIGN KEY(ch_prompt6) REFERENCES characters(id)
);

CREATE TABLE IF NOT EXISTS series(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS characters(
  id INTEGER PRIMARY KEY,
  series_id INTEGER,
  name TEXT NOT NULL,
  FOREIGN KEY(series_id) REFERENCES series(id)
);

CREATE TABLE IF NOT EXISTS tags(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS artists(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  rating INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS artist_combos(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  rating INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS base_prompts(
  id INTEGER PRIMARY KEY,
  start TEXT,
  artists TEXT,
  location TEXT,
  other TEXT,
  quality TEXT,
  negative TEXT,
  nsfw INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS character_prompts(
  id INTEGER PRIMARY KEY,
  ch TEXT,
  outfit TEXT,
  posture TEXT,
  actions TEXT,
  body TEXT,
  other TEXT,
  position TEXT DEFAULT "R2C2",
  uc TEXT -- undesired content
);
