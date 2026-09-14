fn main() -> anyhow::Result<()> {
    ffmpeg_next::init()?;
    println!("rust-playout: native libavcodec {}", ffmpeg_next::codec::version());
    Ok(())
}
