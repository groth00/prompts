use std::{
    error::Error,
    fs::{self},
    io::Cursor,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use rand::distr::{Alphanumeric, SampleString};
use reqwest::{Client, Method, StatusCode, header::HeaderMap};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;
use zip::{read::ZipArchive, result::ZipResult};

const NOVELAI_ENDPOINT: &str = "https://image.novelai.net/ai/generate-image";

use crate::prompt::{CharCaption, Character, NEGATIVE_PROMPT, V4NegativePrompt, V4Prompt};

// TODO: if custom character position, set use_coords to enable

pub async fn generate_image<'a>(
    shape: ImageShape,
    base_prompt: &'a str,
    char_prompts: &'a [Character<'a>],
    negative_prompt: Option<&'a str>,
) -> Result<(), Box<dyn Error>> {
    let mut req = ImageGenRequest::default();

    req.prompt(base_prompt);
    req.height_width(shape);

    req.parameters.seed = rand::random_range(1e9..9e9) as u64;

    if let Some(p) = negative_prompt {
        req.negative_prompt(p);
    }

    for c in char_prompts {
        req.add_character(c);
    }

    let ser = serde_json::to_string_pretty(&req).expect("to_string_pretty");
    println!("{}", ser);

    let r = Requester::default();
    let (bytes, end) = r.call_service(&req).await?;
    println!("{} elapsed", end);

    let res = spawn_blocking(move || -> ZipResult<()> {
        save_image(bytes)?;
        Ok(())
    })
    .await?;
    println!("{:?}", res);

    Ok(())
}

#[derive(Debug)]
pub struct Requester {
    client: Client,
    api_token: String,
}

impl Default for Requester {
    fn default() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .https_only(true)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:139.0) Gecko/20100101 Firefox/139.0")
            .build()
            .expect("build");
        let api_token = std::env::var("NOVELAI_API_KEY").expect("missing api key");
        Self { client, api_token }
    }
}

impl Requester {
    pub async fn call_service<'a>(
        &self,
        params: &ImageGenRequest<'a>,
    ) -> Result<(Bytes, f64), Box<dyn Error>> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());

        let req_id = Alphanumeric.sample_string(&mut rand::rng(), 6);
        headers.insert("x-correlation-id", req_id.parse().unwrap());

        let start = Instant::now();

        let req = self
            .client
            .request(Method::POST, NOVELAI_ENDPOINT)
            .headers(headers)
            .bearer_auth(&self.api_token)
            .json::<ImageGenRequest>(params)
            .build()?;

        let mut attempts = 0;
        let mut wait = 3;
        let resp = loop {
            let resp = self.client.execute(req.try_clone().unwrap()).await?;
            println!("{}", resp.status());

            if resp.status().is_success() {
                break resp;
            } else if resp.status() == StatusCode::TOO_MANY_REQUESTS
                || resp.status().is_server_error()
            {
                if attempts == 0 {
                    panic!("max attempts exceeded");
                }
                tokio::time::sleep(Duration::from_secs(wait)).await;
                wait += 1;
                attempts -= 1;
                println!(
                    "{}: {:?} ({} attempts left)",
                    resp.status(),
                    resp.text().await,
                    attempts
                );
                continue;
            } else if resp.status().is_client_error() {
                panic!("{}: {:?}", resp.status(), resp.text().await);
            }
        };

        let bytes = resp.bytes().await?;
        Ok((bytes, start.elapsed().as_secs_f64()))
    }
}

pub fn save_image(bytes: Bytes) -> ZipResult<()> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)?;
    let mut file = archive.by_index(0)?;

    let mut buf = Vec::with_capacity(file.size() as usize);
    std::io::copy(&mut file, &mut buf)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let output_path = format!("output/{}.png", now);
    fs::write(output_path, &buf)?;

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageGenRequest<'a> {
    action: Action,
    /// base prompt; max length 40_000
    input: &'a str,
    model: Model,
    parameters: RequestParameters<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl<'a> Default for ImageGenRequest<'a> {
    fn default() -> Self {
        Self {
            action: Action::default(),
            input: "",
            model: Model::default(),
            parameters: RequestParameters::default(),
            url: None,
        }
    }
}

impl<'a> ImageGenRequest<'a> {
    fn height_width(&mut self, shape: ImageShape) {
        self.parameters.width = shape.as_width_height().0;
        self.parameters.height = shape.as_width_height().1;
    }

    fn prompt(&mut self, prompt: &'a str) {
        self.input = prompt;
        self.parameters.v4_prompt.caption.base_caption = prompt;
    }

    fn negative_prompt(&mut self, prompt: &'a str) {
        self.parameters.negative_prompt = prompt;
        self.parameters.v4_negative_prompt.caption.base_caption = prompt;
    }

    fn add_character(&mut self, ch: &Character<'a>) {
        self.parameters.character_prompts.push(*ch);
        self.parameters
            .v4_prompt
            .caption
            .char_captions
            .push(CharCaption {
                char_caption: ch.get_prompt(),
                centers: vec![ch.get_center()],
            });
        self.parameters
            .v4_negative_prompt
            .caption
            .char_captions
            .push(CharCaption {
                char_caption: "",
                centers: vec![ch.get_center()],
            })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct RequestParameters<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    add_original_image: Option<bool>,
    #[serde(rename = "autoSmea")]
    auto_smea: bool,
    cfg_rescale: f32,
    #[serde(rename = "characterPrompts")]
    character_prompts: Vec<Character<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_correct: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controlnet_condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controlnet_model: Option<String>,
    controlnet_strength: u8,
    deliberate_euler_ancestral_bug: bool,
    dynamic_thresholding: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_noise_seed: Option<u8>,
    height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    img2img: Option<Img2ImgParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inpaint_img2img2_strength: Option<u8>,
    legacy: bool,
    legacy_uc: bool,
    legacy_v3_extend: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mask: Option<String>,
    n_samples: u8,
    negative_prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    noise: Option<f32>,
    noise_schedule: NoiseSchedule,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalize_reference_strength_multiple: Option<bool>,
    params_version: u8,
    prefer_brownian: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(rename = "qualityToggle")]
    quality_toggle: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_image_multiple: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_information_extracted: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_information_extracted_multiple: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_strength: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_strength_multiple: Option<Vec<f32>>,
    sampler: Sampler,
    scale: f32,
    seed: u64,
    skip_cfg_above_sigma: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sm_dyn: Option<bool>,
    steps: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<Stream>, // msgpack or sse
    #[serde(skip_serializing_if = "Option::is_none")]
    strength: Option<f32>,
    use_coords: bool,
    v4_negative_prompt: V4NegativePrompt<'a>,
    v4_prompt: V4Prompt<'a>,
    width: u32,
}

impl Default for RequestParameters<'_> {
    fn default() -> Self {
        Self {
            cfg_rescale: 0.5,
            character_prompts: vec![],
            controlnet_strength: 1,
            deliberate_euler_ancestral_bug: false,
            dynamic_thresholding: false,
            height: 1216,
            legacy: false,
            legacy_uc: false,
            legacy_v3_extend: false,
            n_samples: 1,
            negative_prompt: NEGATIVE_PROMPT,
            noise_schedule: NoiseSchedule::default(),
            params_version: 3,
            prefer_brownian: true,
            quality_toggle: true,
            sampler: Sampler::default(),
            scale: 5.5,
            seed: 0,
            skip_cfg_above_sigma: None,
            steps: 28,
            use_coords: false,
            v4_prompt: V4Prompt::default(),
            v4_negative_prompt: V4NegativePrompt::default(),
            width: 832,

            // optionals
            add_original_image: None,
            auto_smea: false,
            color_correct: None,
            controlnet_condition: None,
            controlnet_model: None,
            extra_noise_seed: None,
            image: None,
            img2img: None,
            inpaint_img2img2_strength: None,
            mask: None,
            noise: None,
            normalize_reference_strength_multiple: None,
            prompt: None,
            reference_image: None,
            reference_image_multiple: None,
            reference_information_extracted: None,
            reference_information_extracted_multiple: None,
            reference_strength: None,
            reference_strength_multiple: None,
            sm: None,
            sm_dyn: None,
            stream: None,
            strength: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub enum ImageShape {
    #[default]
    Portrait,
    Landscape,
    Square,
    PortraitLarge,
    LandscapeLarge,
    SquareLarge,
    PortraitWallpaper,
    LandscapeWallpaper,
}

impl ImageShape {
    fn as_width_height(&self) -> (u32, u32) {
        match self {
            Self::Portrait => (832, 1216),
            Self::Landscape => (1216, 832),
            Self::Square => (1024, 1024),
            Self::PortraitLarge => (1024, 1536),
            Self::LandscapeLarge => (1536, 1024),
            Self::SquareLarge => (1472, 1472),
            Self::PortraitWallpaper => (1088, 1920),
            Self::LandscapeWallpaper => (1920, 1088),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    Generate,
    Infill,
    Img2Img,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Model {
    #[default]
    #[serde(rename = "nai-diffusion-4-5-full")]
    V45,
    #[serde(rename = "nai-diffusion-4-5-full-inpainting")]
    V45Inpaint,
    // nai-diffusion,
    // safe-diffusion,
    // nai-diffusion-furry,
    // custom,
    // nai-diffusion-inpainting,
    // nai-diffusion-3-inpainting,
    // safe-diffusion-inpainting,
    // furry-diffusion-inpainting,
    // kandinsky-vanilla,
    // nai-diffusion-2,
    // nai-diffusion-3
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NoiseSchedule {
    #[default]
    Karras,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sampler {
    #[default]
    KEulerAncestral,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stream {
    #[default]
    Msgpack,
    Sse,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Img2ImgParameters {
    color_correct: bool,
    extra_noise_seed: u8,
    noise: u64,
    strength: u64,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::prompt::Caption;

    #[test]
    fn serialize() {
        let mut req = ImageGenRequest::default();

        let base_prompt = "nsfw, year 2025, official art, minami (minami373916), asou (asabu202), sky-freedom, pumpkinspicelatte, sp (8454), fellatrix, wakura (gcdan), hth5k, soraoraora, from above, dungeon, empty room, 3::cum in pussy, excessive cum, cum overflow, dripping::, cum pool, 1.16::highly finished, digital illustration, smooth shading, smooth::, 1.1::masterpiece, best quality, incredibly absurdres::, uncensored, -2::multiple views, patreon logo, signature, watermark::, very aesthetic, masterpiece, no text";

        req.prompt(base_prompt);
        req.height_width(ImageShape::Portrait);
        req.parameters.scale = 5.5;
        req.parameters.seed = 243998974;

        assert_eq!(
            req.parameters.v4_negative_prompt,
            V4NegativePrompt {
                caption: Caption {
                    base_caption: NEGATIVE_PROMPT,
                    char_captions: vec![],
                },
                legacy_uc: false,
            }
        );
    }
}
