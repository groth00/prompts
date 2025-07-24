use std::{
    fmt::{self, Display},
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

#[derive(Debug, Clone)]
pub enum ImageGenerationError {
    FailedAfterMaxAttempts,
    SendRequest(String),
    ClientError(String),
    Deserialization(String),
    ZipError(String),
    JoinError,
}

impl Display for ImageGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ImageGenerationError::*;
        match self {
            FailedAfterMaxAttempts => write!(f, "failed to generate after max attempts"),
            JoinError => write!(
                f,
                "something went wrong with the background task to save the image"
            ),
            SendRequest(err) => write!(f, "{}", err),
            ClientError(err) => write!(f, "{}", err),
            Deserialization(err) => write!(f, "read response bytes: {}", err),
            ZipError(err) => write!(f, "zip: {}", err),
        }
    }
}

impl Requester {
    pub async fn generate_image<'a>(
        &self,
        shape: ImageShape,
        mut req: ImageGenRequest<'a>,
    ) -> Result<String, ImageGenerationError> {
        req.height_width(shape);
        req.parameters.seed = rand::random_range(1e9..9e9) as u64;

        let (bytes, end) = self.call_service(&req).await?;
        println!("{} elapsed", end);

        let res = spawn_blocking(move || -> Result<String, ImageGenerationError> {
            let output_path =
                save_image(bytes).map_err(|e| ImageGenerationError::ZipError(e.to_string()))?;
            Ok(output_path)
        })
        .await
        .map_err(|_e| ImageGenerationError::JoinError)??;
        println!("{:?}", res);

        Ok(res)
    }

    pub async fn call_service<'a>(
        &self,
        params: &ImageGenRequest<'a>,
    ) -> Result<(Bytes, f64), ImageGenerationError> {
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
            .build()
            .expect("failed to build request");

        let mut attempts = 0;
        let mut wait = 3;
        let resp = loop {
            let resp = self
                .client
                .execute(req.try_clone().unwrap())
                .await
                .map_err(|e| ImageGenerationError::SendRequest(e.to_string()))?;
            println!("{}", resp.status());

            if resp.status().is_success() {
                break resp;
            } else if resp.status() == StatusCode::TOO_MANY_REQUESTS
                || resp.status().is_server_error()
            {
                if attempts == 0 {
                    return Err(ImageGenerationError::FailedAfterMaxAttempts);
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
                return Err(ImageGenerationError::ClientError(format!(
                    "{}: {:?}",
                    resp.status(),
                    resp.text().await
                )));
            }
        };

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ImageGenerationError::Deserialization(e.to_string()))?;

        Ok((bytes, start.elapsed().as_secs_f64()))
    }
}

pub fn save_image(bytes: Bytes) -> ZipResult<String> {
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
    fs::write(&output_path, &buf)?;

    Ok(output_path)
}

#[derive(Serialize, Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
pub struct ImageGenRequest<'a> {
    action: Action,
    /// base prompt; max length 40_000
    input: String,
    model: Model,
    parameters: RequestParameters<'a>,
}

impl<'a> Default for ImageGenRequest<'a> {
    fn default() -> Self {
        Self {
            action: Action::default(),
            input: String::new(),
            model: Model::default(),
            parameters: RequestParameters::default(),
        }
    }
}

impl<'a> ImageGenRequest<'a> {
    pub fn height_width(&mut self, shape: ImageShape) {
        self.parameters.width = shape.as_width_height().0;
        self.parameters.height = shape.as_width_height().1;
    }

    pub fn prompt(&mut self, prompt: String) {
        self.input = prompt.clone();
        self.parameters.v4_prompt.caption.base_caption = prompt;
    }

    pub fn add_character(&mut self, ch: &Character) {
        self.parameters.character_prompts.push(ch.clone());
        self.parameters
            .v4_prompt
            .caption
            .char_captions
            .push(CharCaption {
                char_caption: ch.get_prompt().to_string(),
                centers: vec![ch.get_center()],
            });
        self.parameters
            .v4_negative_prompt
            .caption
            .char_captions
            .push(CharCaption {
                char_caption: String::from(""),
                centers: vec![ch.get_center()],
            })
    }

    pub fn use_coords(&mut self, enable: bool) {
        self.parameters.use_coords = enable;
        self.parameters.v4_prompt.use_coords = enable;
    }
}

#[derive(Serialize, Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct RequestParameters<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    add_original_image: Option<bool>,
    #[serde(rename = "autoSmea")]
    auto_smea: bool,
    cfg_rescale: f32,
    #[serde(rename = "characterPrompts")]
    character_prompts: Vec<Character>,
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
    v4_negative_prompt: V4NegativePrompt,
    v4_prompt: V4Prompt,
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

#[allow(dead_code)]
#[derive(Default, Clone, Copy)]
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

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    Generate,
    Infill,
    Img2Img,
}

#[derive(Default, Serialize, Deserialize)]
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

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NoiseSchedule {
    #[default]
    Karras,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sampler {
    #[default]
    KEulerAncestral,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stream {
    #[default]
    Msgpack,
    Sse,
}

#[derive(Default, Serialize, Deserialize)]
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

        let base_prompt = "nsfw, year 2025, official art, minami (minami373916), asou (asabu202), sky-freedom, pumpkinspicelatte, sp (8454), fellatrix, wakura (gcdan), hth5k, soraoraora, from above, dungeon, empty room, 3::cum in pussy, excessive cum, cum overflow, dripping::, cum pool, 1.16::highly finished, digital illustration, smooth shading, smooth::, 1.1::masterpiece, best quality, incredibly absurdres::, uncensored, -2::multiple views, patreon logo, signature, watermark::, very aesthetic, masterpiece, no text".into();

        req.prompt(base_prompt);
        req.height_width(ImageShape::Portrait);
        req.parameters.scale = 5.5;
        req.parameters.seed = 243998974;

        assert_eq!(
            req.parameters.v4_negative_prompt,
            V4NegativePrompt {
                caption: Caption {
                    base_caption: String::from(NEGATIVE_PROMPT),
                    char_captions: vec![],
                },
                legacy_uc: false,
            }
        );
    }
}
