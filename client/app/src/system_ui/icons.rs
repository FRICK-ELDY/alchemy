//! Load static Material-style PNG icons from `assets/icons/`.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};

#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn load_bundled() -> Result<HashMap<&'static str, RgbaImage>> {
    let mut map = HashMap::new();
    for (name, bytes) in [
        (
            "mic",
            include_bytes!("../../assets/icons/mic.png").as_slice(),
        ),
        (
            "mic_off",
            include_bytes!("../../assets/icons/mic_off.png").as_slice(),
        ),
        (
            "volume_up",
            include_bytes!("../../assets/icons/volume_up.png").as_slice(),
        ),
        (
            "hearing",
            include_bytes!("../../assets/icons/hearing.png").as_slice(),
        ),
        (
            "sync",
            include_bytes!("../../assets/icons/sync.png").as_slice(),
        ),
        (
            "close",
            include_bytes!("../../assets/icons/close.png").as_slice(),
        ),
        (
            "power",
            include_bytes!("../../assets/icons/power.png").as_slice(),
        ),
        (
            "settings",
            include_bytes!("../../assets/icons/settings.png").as_slice(),
        ),
        (
            "logout",
            include_bytes!("../../assets/icons/logout.png").as_slice(),
        ),
        (
            "exit_to_app",
            include_bytes!("../../assets/icons/exit_to_app.png").as_slice(),
        ),
    ] {
        map.insert(
            name,
            decode_png(bytes).with_context(|| format!("decode {name}"))?,
        );
    }
    Ok(map)
}

fn decode_png(bytes: &[u8]) -> Result<RgbaImage> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().context("png read_info")?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).context("png next_frame")?;
    let width = info.width;
    let height = info.height;
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => {
            let rgb = &buf[..info.buffer_size()];
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for chunk in rgb.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            rgba
        }
        other => bail!("unsupported png color type: {other:?}"),
    };
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
