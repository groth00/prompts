use serde::{Deserialize, Serialize};

const START_TAGS: &'static str = "year 2025, official art";
const QUALITY_TAGS: &'static str = "1.16::highly finished, digital illustration, smooth shading, smooth::, 1.1::masterpiece, best quality, incredibly absurdres::, uncensored";
const NEGATIVE_TAGS: &'static str = "-2::multiple views, patreon logo, signature watermark::";
pub const NEGATIVE_PROMPT: &str = "lowres, artistic error, film grain, scan artifacts, worst quality, bad quality, jpeg artifacts, very displeasing, chromatic aberration, dithering, halftone, screentone, multiple views, logo, too many watermarks, negative space, blank page, blurry, lowres, error, film grain, scan artifacts, worst quality, bad quality, jpeg artifacts, very displeasing, chromatic aberration, logo, too many watermarks, {{{bad eyes}}}, blurry eyes, fewer, extra, missing, worst quality, watermark, unfinished, displeasing, signature, extra digits, artistic error, username, scan, bad anatomy, @_@, mismatched pupils, heart-shaped pupils, glowing eyes, low quality, {{{bad}}}, normal quality, disfigured, flower, artist signature, watermark, monochrome, black bars, cinematic bars, plaque, wall ornament, speech bubble, extra arms, extra breasts, loli, child, amputee, missing limb, 1.22::extra fingers, long fingers, missing fingers, bad hands::, extra digit, fewer digits, mutation, white border, eyes without pupils, multiple views, 1.3::disembodied penis::, x-ray, fake animal ears, animal ears, 1.1::pubic hair, female pubic hair, male pubic hair::, censored, border, 1.2::sound effects, text::";

trait Prompt {
    fn build(&self) -> String;
}

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

impl Prompt for BasePrompt<'_> {
    fn build(&self) -> String {
        let rating = if self.nsfw { "nsfw" } else { "sfw" };
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

pub struct FemaleCharacterPrompt<'a> {
    pub ch: &'a str,
    pub outfit: &'a str,
    pub posture: &'a str,
    pub actions: &'a str,
    pub body: &'a str,
    pub other: &'a str,
}

impl Prompt for FemaleCharacterPrompt<'_> {
    fn build(&self) -> String {
        String::new()
    }
}

pub struct MaleCharacterPrompt<'a> {
    pub ch: &'a str,
    pub actions: &'a str,
    pub pp: &'a str,
    pub other: &'a str,
}

impl Prompt for MaleCharacterPrompt<'_> {
    fn build(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Character<'a> {
    pub prompt: &'a str,
    pub uc: &'a str,
    pub center: Point,
    enabled: bool,
}

impl<'a> Default for Character<'a> {
    fn default() -> Self {
        Self {
            prompt: "",
            uc: "",
            center: Point::default(),
            enabled: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
pub struct V4Prompt<'a> {
    pub caption: Caption<'a>,
    pub use_coords: bool,
    pub use_order: bool,
}

impl<'a> Default for V4Prompt<'a> {
    fn default() -> Self {
        Self {
            caption: Caption::default(),
            use_coords: false,
            use_order: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(bound(deserialize = "'de: 'a"))]
pub struct V4NegativePrompt<'a> {
    pub caption: Caption<'a>,
    pub legacy_uc: bool,
}

impl<'a> Default for V4NegativePrompt<'a> {
    fn default() -> Self {
        Self {
            caption: Caption {
                base_caption: NEGATIVE_PROMPT,
                char_captions: vec![],
            },
            legacy_uc: false,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Caption<'a> {
    pub base_caption: &'a str,
    pub char_captions: Vec<CharCaption<'a>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CharCaption<'a> {
    pub char_caption: &'a str,
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
