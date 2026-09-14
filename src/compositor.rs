use crate::model::{HEIGHT, WIDTH};
use anyhow::Result;
use ffmpeg_next as av;
use font8x8::UnicodeFonts;

pub fn canvas(y: u8, u: u8, v: u8) -> av::frame::Video {
    let mut frame = av::frame::Video::new(av::format::Pixel::YUV420P, WIDTH, HEIGHT);
    for (p, value) in [y, u, v].into_iter().enumerate() {
        frame.data_mut(p).fill(value);
    }
    frame
}

pub fn text(frame: &mut av::frame::Video, x: usize, y: usize, label: &str, scale: usize) {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    for (i, c) in label.chars().enumerate() {
        let Some(glyph) = font8x8::BASIC_FONTS.get(c) else {
            continue;
        };
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = x + i * 9 * scale + col * scale + dx;
                        let py = y + row * scale + dy;
                        if px < width && py < height {
                            let stride = frame.stride(0);
                            frame.data_mut(0)[py * stride + px] = 235;
                            for p in 1..3 {
                                let s = frame.stride(p);
                                frame.data_mut(p)[(py / 2) * s + px / 2] = 128;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn slate(tick: u64) -> av::frame::Video {
    let mut frame = canvas(29, 137, 121);
    text(&mut frame, 90, 270, "RUST / PLAYOUT", 7);
    text(
        &mut frame,
        94,
        365,
        "CHANNEL READY - ADD MEDIA TO GO ON AIR",
        2,
    );
    text(
        &mut frame,
        94,
        420,
        &format!(
            "PROGRAM  {:02}:{:02}:{:02}",
            tick / 108000,
            tick / 1800 % 60,
            tick / 30 % 60
        ),
        3,
    );
    frame
}

pub fn paste(dst: &mut av::frame::Video, src: &av::frame::Video, x: usize, y: usize) {
    for p in 0..3 {
        let div = if p == 0 { 1 } else { 2 };
        let w = src.width() as usize / div;
        let h = src.height() as usize / div;
        let ds = dst.stride(p);
        let ss = src.stride(p);
        for row in 0..h {
            let offset = (y / div + row) * ds + x / div;
            dst.data_mut(p)[offset..offset + w]
                .copy_from_slice(&src.data(p)[row * ss..row * ss + w]);
        }
    }
}

pub struct Compositor {
    shrink: av::software::scaling::Context,
}

impl Compositor {
    pub fn new() -> Result<Self> {
        Ok(Self {
            shrink: av::software::scaling::Context::get(
                av::format::Pixel::YUV420P,
                WIDTH,
                HEIGHT,
                av::format::Pixel::YUV420P,
                1024,
                576,
                av::software::scaling::Flags::BILINEAR,
            )?,
        })
    }

    pub fn banner(
        &mut self,
        program: &av::frame::Video,
        artwork: Option<&av::frame::Video>,
        title: &str,
        remaining_ms: u64,
    ) -> Result<av::frame::Video> {
        let mut out = artwork.cloned().unwrap_or_else(|| canvas(66, 125, 112));
        let mut reduced = av::frame::Video::empty();
        self.shrink.run(program, &mut reduced)?;
        paste(&mut out, &reduced, 0, 0);
        text(
            &mut out,
            36,
            610,
            title,
            if title.len() > 44 { 2 } else { 3 },
        );
        text(&mut out, 36, 664, "SPONSORED  /  PROGRAM CONTINUES", 2);
        text(&mut out, 1060, 255, "BACK IN", 2);
        text(
            &mut out,
            1060,
            300,
            &format!("{:03}", remaining_ms.div_ceil(1000)),
            6,
        );
        text(&mut out, 1060, 363, "SECONDS", 2);
        Ok(out)
    }
}

pub fn artwork(path: &std::path::Path) -> Result<av::frame::Video> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let rgb = reader
        .decode()?
        .resize_exact(WIDTH, HEIGHT, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let mut input = av::frame::Video::new(av::format::Pixel::RGB24, WIDTH, HEIGHT);
    let stride = input.stride(0);
    for y in 0..HEIGHT as usize {
        input.data_mut(0)[y * stride..y * stride + WIDTH as usize * 3]
            .copy_from_slice(&rgb.as_raw()[y * WIDTH as usize * 3..(y + 1) * WIDTH as usize * 3]);
    }
    let mut scaler = av::software::scaling::Context::get(
        av::format::Pixel::RGB24,
        WIDTH,
        HEIGHT,
        av::format::Pixel::YUV420P,
        WIDTH,
        HEIGHT,
        av::software::scaling::Flags::BILINEAR,
    )?;
    let mut output = av::frame::Video::empty();
    scaler.run(&input, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn banner_changes_encoded_canvas_and_preserves_program_region() {
        av::init().unwrap();
        let program = canvas(120, 128, 128);
        let out = Compositor::new()
            .unwrap()
            .banner(&program, None, "TEST", 2000)
            .unwrap();
        assert_eq!(out.data(0)[100 * out.stride(0) + 100], 120);
        assert_eq!(out.data(0)[700 * out.stride(0) + 1250], 66);
        assert_eq!(program.data(0)[700 * program.stride(0) + 1250], 120);
    }
}
