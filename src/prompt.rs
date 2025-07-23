use serde::{Deserialize, Serialize};

const START_TAGS: &'static str = "year 2025, official art";
const QUALITY_TAGS: &'static str = "1.16::highly finished, digital illustration, smooth shading, smooth::, 1.1::masterpiece, best quality, incredibly absurdres::, uncensored";
const NEGATIVE_TAGS: &'static str = "-2::multiple views, patreon logo, signature watermark::";
pub const NEGATIVE_PROMPT: &str = "lowres, artistic error, film grain, scan artifacts, worst quality, bad quality, jpeg artifacts, very displeasing, chromatic aberration, dithering, halftone, screentone, multiple views, logo, too many watermarks, negative space, blank page, blurry, lowres, error, film grain, scan artifacts, worst quality, bad quality, jpeg artifacts, very displeasing, chromatic aberration, logo, too many watermarks, {{{bad eyes}}}, blurry eyes, fewer, extra, missing, worst quality, watermark, unfinished, displeasing, signature, extra digits, artistic error, username, scan, bad anatomy, @_@, mismatched pupils, heart-shaped pupils, glowing eyes, low quality, {{{bad}}}, normal quality, disfigured, flower, artist signature, watermark, monochrome, black bars, cinematic bars, plaque, wall ornament, speech bubble, extra arms, extra breasts, loli, child, amputee, missing limb, 1.22::extra fingers, long fingers, missing fingers, bad hands::, extra digit, fewer digits, mutation, white border, eyes without pupils, multiple views, 1.3::disembodied penis::, x-ray, fake animal ears, animal ears, 1.1::pubic hair, female pubic hair, male pubic hair::, censored, border, 1.2::sound effects, text::";

pub struct BasePrompt<'a> {
    pub start: &'a str,
    pub artists: &'a str,
    pub location: &'a str,
    pub other: &'a str,
    pub quality: &'a str,
    pub negative: &'a str,
    pub nsfw: bool,
}

impl Default for BasePrompt<'_> {
    fn default() -> Self {
        Self {
            start: START_TAGS,
            artists: "",
            location: "",
            other: "",
            quality: QUALITY_TAGS,
            negative: NEGATIVE_TAGS,
            nsfw: true,
        }
    }
}

impl BasePrompt<'_> {
    fn build(&self) -> String {
        let rating = if self.nsfw { "nsfw" } else { "rating:general" };
        [
            rating,
            self.start,
            self.artists,
            self.location,
            self.other,
            self.quality,
            self.negative,
        ]
        .iter()
        .fold(String::new(), |mut acc, s| {
            if !s.is_empty() {
                acc.push_str(s);
                acc.push(',');
            }
            acc
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    prompt: String,
    center: Point,
    enabled: bool,
}

impl Character {
    pub fn new() -> Self {
        Self {
            prompt: String::new(),
            center: Point::default(),
            enabled: true,
        }
    }

    pub fn prompt(&mut self, s: String) -> &mut Self {
        self.prompt = s;
        self
    }

    pub fn center(&mut self, pos: Position) -> &mut Self {
        self.center = pos.as_point();
        self
    }

    pub fn finish(&self) -> Self {
        self.clone()
    }

    pub fn get_prompt(&self) -> &str {
        &self.prompt
    }

    pub const fn get_center(&self) -> Point {
        self.center
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct V4Prompt {
    pub caption: Caption,
    pub use_coords: bool,
    pub use_order: bool,
}

impl Default for V4Prompt {
    fn default() -> Self {
        Self {
            caption: Caption::default(),
            use_coords: false,
            use_order: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct V4NegativePrompt {
    pub caption: Caption,
    pub legacy_uc: bool,
}

impl Default for V4NegativePrompt {
    fn default() -> Self {
        Self {
            caption: Caption {
                base_caption: String::from(NEGATIVE_PROMPT),
                char_captions: vec![],
            },
            legacy_uc: false,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Caption {
    pub base_caption: String,
    pub char_captions: Vec<CharCaption>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CharCaption {
    pub char_caption: String,
    pub centers: Vec<Point>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Default for Point {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum Position {
    R0C0,
    R0C1,
    R0C2,
    R0C3,
    R0C4,
    R1C0,
    R1C1,
    R1C2,
    R1C3,
    R1C4,
    R2C0,
    R2C1,
    R2C2,
    R2C3,
    R2C4,
    R3C0,
    R3C1,
    R3C2,
    R3C3,
    R3C4,
    R4C0,
    R4C1,
    R4C2,
    R4C3,
    R4C4,
}

impl Position {
    pub const fn as_point(&self) -> Point {
        match self {
            Self::R0C0 => Point { x: 0.1, y: 0.1 },
            Self::R0C1 => Point { x: 0.1, y: 0.3 },
            Self::R0C2 => Point { x: 0.1, y: 0.5 },
            Self::R0C3 => Point { x: 0.1, y: 0.7 },
            Self::R0C4 => Point { x: 0.1, y: 0.9 },
            Self::R1C0 => Point { x: 0.3, y: 0.1 },
            Self::R1C1 => Point { x: 0.3, y: 0.3 },
            Self::R1C2 => Point { x: 0.3, y: 0.5 },
            Self::R1C3 => Point { x: 0.3, y: 0.7 },
            Self::R1C4 => Point { x: 0.3, y: 0.9 },
            Self::R2C0 => Point { x: 0.5, y: 0.1 },
            Self::R2C1 => Point { x: 0.5, y: 0.3 },
            Self::R2C2 => Point { x: 0.5, y: 0.5 },
            Self::R2C3 => Point { x: 0.5, y: 0.7 },
            Self::R2C4 => Point { x: 0.5, y: 0.9 },
            Self::R3C0 => Point { x: 0.7, y: 0.1 },
            Self::R3C1 => Point { x: 0.7, y: 0.3 },
            Self::R3C2 => Point { x: 0.7, y: 0.5 },
            Self::R3C3 => Point { x: 0.7, y: 0.7 },
            Self::R3C4 => Point { x: 0.7, y: 0.9 },
            Self::R4C0 => Point { x: 0.9, y: 0.1 },
            Self::R4C1 => Point { x: 0.9, y: 0.3 },
            Self::R4C2 => Point { x: 0.9, y: 0.5 },
            Self::R4C3 => Point { x: 0.9, y: 0.7 },
            Self::R4C4 => Point { x: 0.9, y: 0.9 },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn build() {
        let mut p = BasePrompt::default();
        let artists = "minami (minami373916), asou (asabu202), sky-freedom, pumpkinspicelatte, sp (8454), fellatrix, wakura (gcdan), hth5k, soraoraora";
        p.artists = artists;

        let full = p.build();
        println!("{}", full);

        let mut expected = ["nsfw", START_TAGS, artists, QUALITY_TAGS, NEGATIVE_TAGS].join(",");
        expected.push(',');

        assert_eq!(full, expected);
    }
}
