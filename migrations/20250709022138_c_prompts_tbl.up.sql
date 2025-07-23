-- Add up migration script here
-- NOTE: not in use
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
  name TEXT NOT NULL,
  category TEXT NOT NULL
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
  prompt_start INTEGER,
  artists INTEGER,
  location INTEGER,
  scene INTEGER,
  quality INTEGER,
  negative INTEGER,
  nsfw INTEGER DEFAULT 1,
  FOREIGN KEY(prompt_start) REFERENCES prompt_starts(id),
  FOREIGN KEY(artists) REFERENCES artist_combos(id),
  FOREIGN KEY(location) REFERENCES locations(id),
  FOREIGN KEY(scene) REFERENCES scenes(id),
  FOREIGN KEY(quality) REFERENCES quality(id),
  FOREIGN KEY(negative) REFERENCES negatives(id)
);

CREATE TABLE IF NOT EXISTS prompt_starts(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS locations(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS scenes(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS quality(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS negatives(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS character_prompts(
  id INTEGER PRIMARY KEY,
  ch INTEGER,
  ch_desc INTEGER,
  outfit INTEGER,
  posture INTEGER,
  actions INTEGER,
  body INTEGER,
  expression INTEGER,
  position TEXT DEFAULT "R2C2",
  uc TEXT, -- undesired content
  FOREIGN KEY(ch) REFERENCES characters(id),
  FOREIGN KEY(ch_desc) REFERENCES character_desc(id),
  FOREIGN KEY(outfit) REFERENCES outfits(id),
  FOREIGN KEY(posture) REFERENCES postures(id),
  FOREIGN KEY(actions) REFERENCES actions(id),
  FOREIGN KEY(body) REFERENCES body(id),
  FOREIGN KEY(expression) REFERENCES expressions(id)
);

CREATE TABLE IF NOT EXISTS characters_desc(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS outfits(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS postures(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS actions(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS body(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS expressions(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  content TEXT NOT NULL
);
