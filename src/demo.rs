use crate::{api::save_asset, compositor, model::*, output::Output, source::probe};
use anyhow::Result;
use std::path::Path;

/// Generate actual H.264/AAC fixtures entirely through linked libraries.
pub fn generate(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root.join("media"))?;
    for (n, (name, y, u, v, frequency)) in [
        ("01 - Studio / Opening", 64, 143, 111, 220.0),
        ("02 - Field / Blue hour", 84, 158, 104, 330.0),
        ("03 - Break / Golden hour", 126, 91, 156, 440.0),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("demo-{}", n + 1);
        let path = root.join("media").join(format!("{id}.mp4"));
        if path.exists() {
            continue;
        }
        tracing::info!("Generating {name}");
        let mut output = Output::new(&path, false)?;
        for tick in 0..FPS * 15 {
            let mut picture = compositor::canvas(y, u, v);
            compositor::text(&mut picture, 80, 100, "RUST / PLAYOUT", 3);
            compositor::text(&mut picture, 80, 260, &name[5..], 5);
            compositor::text(
                &mut picture,
                80,
                365,
                "LIVE SOURCES. ONE CONTINUOUS CHANNEL.",
                2,
            );
            compositor::text(
                &mut picture,
                80,
                450,
                &format!(
                    "CLIP {:02}   {:02}.{:02} / 15.00",
                    n + 1,
                    tick / FPS,
                    tick % FPS
                ),
                3,
            );
            let stride = picture.stride(0);
            let width = ((tick + 1) as usize * 1120 / (FPS * 15) as usize).max(1);
            for row in 565..579 {
                picture.data_mut(0)[row * stride + 80..row * stride + 80 + width].fill(220);
            }
            let samples: Vec<f32> = (0..SAMPLES_PER_TICK)
                .map(|i| {
                    let t = (tick * SAMPLES_PER_TICK as u64 + i as u64) as f64 / RATE as f64;
                    ((t * frequency * std::f64::consts::TAU).sin() * 0.04) as f32
                })
                .collect();
            output.write(&mut picture, tick, &[samples.clone(), samples])?;
        }
        output.finish()?;
        let asset = probe(&path, id, name.into())?;
        save_asset(root, &asset)?;
    }
    Ok(())
}
