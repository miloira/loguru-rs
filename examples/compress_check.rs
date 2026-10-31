//! Verifies gzip rotation. Run with: `cargo run --features compression --example compress_check`

use loguru::{Compression, FileSink, Logger, Rotation};

fn main() -> std::io::Result<()> {
    let dir = std::env::temp_dir().join("loguru_compress_check");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("test.log");

    let log = Logger::builder()
        .name("c")
        .sink(
            FileSink::new(&path)?
                .with_rotation(Rotation::Size(200))
                .with_compression(Compression::Gzip),
        )
        .build();

    for i in 0..50 {
        log.info(format!("line number {i} with some padding text to grow the file"));
    }
    log.flush();

    let mut gz = 0;
    for entry in std::fs::read_dir(&dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".gz") {
            gz += 1;
        }
    }
    println!("gzipped rotated files found: {gz}");
    assert!(gz > 0, "expected at least one .gz rotated file");
    println!("OK");
    Ok(())
}
