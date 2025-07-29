use std::{error::Error, io::Read};

use flate2::read::GzDecoder;
use image::DynamicImage;
use ndarray::Array2;
use serde_json::{Map, Value};

const MAGIC: &str = "stealth_pngcomp";

struct LSBExtractor {
    data: Vec<u8>,
    pos: usize,
}

impl LSBExtractor {
    fn new(im: DynamicImage) -> Self {
        let data = byteize(im).expect("something went wrong");

        Self { data, pos: 0 }
    }

    fn read_n(&mut self, n: usize) -> &[u8] {
        let bytes = &self.data[self.pos..self.pos + n];
        self.pos += n;
        &bytes
    }

    fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_n(4);
        if bytes.len() == 4 {
            Some(u32::from_be_bytes(bytes.try_into().unwrap()))
        } else {
            None
        }
    }
}

fn byteize(im: DynamicImage) -> Result<Vec<u8>, Box<dyn Error>> {
    let img = im.to_rgba8();

    let (width, height) = img.dimensions();
    let mut alpha = Array2::<u8>::zeros((height as usize, width as usize));

    for (x, y, pixel) in img.enumerate_pixels() {
        alpha[[y as usize, x as usize]] = pixel[3];
    }

    let transposed = alpha.t();
    let mut flat: Vec<u8> = transposed.iter().copied().collect();

    let trunc_len = (flat.len() / 8) * 8;
    flat.truncate(trunc_len);

    for v in &mut flat {
        *v &= 1;
    }

    let mut packed = Vec::with_capacity(flat.len() / 8);
    for chunk in flat.chunks(8) {
        let mut byte = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            byte |= bit << (7 - i);
        }
        packed.push(byte);
    }

    Ok(packed)
}

pub fn extract_image_metadata(im: DynamicImage) -> Result<Map<String, Value>, Box<dyn Error>> {
    let mut reader = LSBExtractor::new(im);
    let magic = String::from_utf8(reader.read_n(MAGIC.len()).to_vec()).expect("invalid utf-8");
    assert_eq!(magic, MAGIC);

    if let Some(read_len) = reader.read_u32() {
        let json_data = reader.read_n(read_len as usize);
        let mut s = String::with_capacity(json_data.len());

        let mut gz = GzDecoder::new(json_data);
        gz.read_to_string(&mut s)?;

        let mut val: Value = serde_json::from_str(&s)?;
        if let Some(map) = val.as_object_mut() {
            if let Some(comment) = map.get("Comment") {
                map.insert(
                    "Comment".into(),
                    serde_json::from_str(comment.as_str().unwrap())?,
                );
            }
            Ok(map.clone())
        } else {
            panic!("map");
        }
    } else {
        panic!("read_len");
    }
}
