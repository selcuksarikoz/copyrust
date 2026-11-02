use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use copyrust::disk;
use copyrust::prompt::{menu, read_line, select_extensions, source_folders_with};

#[derive(Clone, Copy)]
enum Format {
    Jpg,
    Png,
    Webp,
    Avif,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
        }
    }
}

struct Job {
    source: PathBuf,
    destination: PathBuf,
}

fn main() -> ExitCode {
    let mut failed = false;
    loop {
        match run() {
            Ok(()) => failed = false,
            Err(error) if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof) => {
                return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
            }
            Err(error) => {
                failed = true;
                eprintln!("error: {error}");
            }
        }
        println!("\nReady for another job. Enter a source folder or q to quit.\n");
    }
}

fn run() -> io::Result<()> {
    println!("=== optimizer ===\n");
    let input = source_folders_with(|path| !path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.starts_with("._") || name.starts_with(".optimizer-") || name == ".system"
    }))?;
    let images: Vec<PathBuf> = input.files.into_iter().filter(|path| supported(path)).collect();
    if images.is_empty() {
        println!("No JPG, PNG, WebP, or AVIF images found.");
        return Ok(());
    }
    let picked = select_extensions(&images)?;
    let selected: Vec<PathBuf> = picked.into_iter().map(|i| images[i].clone()).collect();
    println!("\n{} image(s) selected.\n", selected.len());

    let format = match menu("Output format?", &["JPG", "PNG", "WebP", "AVIF"], 2)? {
        0 => Format::Jpg,
        1 => Format::Png,
        2 => Format::Webp,
        _ => Format::Avif,
    };
    let quality = quality_prompt(format)?;
    let delete_original = menu(
        "After successful conversion, delete the original?",
        &["Yes, delete original", "No, keep original"],
        0,
    )? == 0;
    check_tools(format)?;

    let jobs = plan_jobs(&selected, format, delete_original);
    let solid_state = input.dirs.iter().map(|dir| disk::detect(dir))
        .collect::<io::Result<Vec<_>>>()?.iter().all(|disk| disk.solid_state);
    let workers = (if solid_state { 4 } else { 2 }).min(jobs.len());
    println!("Processing with {workers} worker(s)...\n");
    let mut converted = 0;
    let mut deleted = 0;
    let mut failed = 0;
    let mut before = 0u64;
    let mut after = 0u64;
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| -> io::Result<()> {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            let jobs = &jobs;
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= jobs.len() { break; }
                if sender.send((i, convert(&jobs[i], format, quality, delete_original, i))).is_err() {
                    break;
                }
            });
        }
        drop(sender);
        let mut last_draw = Instant::now();
        for (done, (i, result)) in receiver.into_iter().enumerate() {
            match result {
                Ok((old_size, new_size, removed)) => {
                    converted += 1;
                    deleted += usize::from(removed);
                    before += old_size;
                    after += new_size;
                }
                Err(error) => {
                    failed += 1;
                    eprintln!("\nFAILED {}: {error}", jobs[i].source.display());
                }
            }
            if last_draw.elapsed() >= Duration::from_millis(100) || done + 1 == jobs.len() {
                print!("\rProcessed {}/{} image(s); failed: {} ", done + 1, jobs.len(), failed);
                io::stdout().flush()?;
                last_draw = Instant::now();
            }
        }
        Ok(())
    })?;
    println!("\nDone. converted: {converted}, originals deleted: {deleted}, failed: {failed}, input: {before} bytes, output: {after} bytes");
    println!("{}", space_summary(before, after, delete_original));
    if failed > 0 { Err(io::Error::other("some images could not be converted")) } else { Ok(()) }
}

fn space_summary(before: u64, after: u64, delete_original: bool) -> String {
    if delete_original {
        let change = before.abs_diff(after);
        let percent = if before == 0 { 0.0 } else { change as f64 * 100.0 / before as f64 };
        if before >= after {
            format!("Estimated space saved: {} ({percent:.1}%)", size_label(change))
        } else {
            format!("Estimated extra space used: {} ({percent:.1}%)", size_label(change))
        }
    } else {
        format!("Additional space used: {} (originals kept)", size_label(after))
    }
}

fn size_label(bytes: u64) -> String {
    if bytes < 1024 { return format!("{bytes} B"); }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB"] {
        value /= 1024.0;
        unit = next;
        if value < 1024.0 { break; }
    }
    format!("{value:.2} {unit}")
}

fn supported(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif"))
        .unwrap_or(false)
}

fn quality_prompt(format: Format) -> io::Result<u8> {
    if matches!(format, Format::Png) {
        println!("PNG is lossless; this value controls compression effort, not visual quality.");
    }
    loop {
        print!("Quality 1-100 [default: 80]: ");
        let value = read_line()?;
        if value.is_empty() { return Ok(80); }
        if let Ok(quality @ 1..=100) = value.parse::<u8>() { return Ok(quality); }
        println!("Enter a number from 1 to 100.");
    }
}

fn check_tools(format: Format) -> io::Result<()> {
    for tool in ["ffmpeg", "ffprobe"] {
        check_tool(tool)?;
    }
    match format {
        Format::Webp => check_tool("cwebp")?,
        Format::Avif => check_tool("avifenc")?,
        _ => {}
    }
    Ok(())
}

fn check_tool(tool: &str) -> io::Result<()> {
    if Command::new(tool).arg("-version").output().is_err() {
        return Err(io::Error::other(format!("{tool} is required but not installed")));
    }
    Ok(())
}

fn plan_jobs(selected: &[PathBuf], format: Format, delete_original: bool) -> Vec<Job> {
    let mut reserved = HashSet::new();
    selected.iter().map(|source| {
        let same_format = source.extension().and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case(format.extension())).unwrap_or(false);
        let destination = if same_format && delete_original {
            source.clone()
        } else {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let base = if same_format { format!("{stem} optimized") } else { stem.to_string() };
            let parent = source.parent().unwrap_or(Path::new("."));
            let mut candidate = parent.join(format!("{base}.{}", format.extension()));
            let mut n = 1;
            while candidate.exists() || reserved.contains(&path_key(&candidate)) {
                candidate = parent.join(format!("{base} ({n}).{}", format.extension()));
                n += 1;
            }
            candidate
        };
        reserved.insert(path_key(&destination));
        Job { source: source.clone(), destination }
    }).collect()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn convert(job: &Job, format: Format, quality: u8, delete_original: bool, index: usize) -> io::Result<(u64, u64, bool)> {
    let old_size = fs::metadata(&job.source)?.len();
    let dimensions = probe(&job.source, true)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let temporary = job.source.parent().unwrap_or(Path::new(".")).join(format!(
        ".optimizer-{}-{stamp}-{index}.{}", std::process::id(), format.extension()
    ));
    let intermediate = temporary.with_extension("input.png");
    let result = (|| {
        let input = if matches!(format, Format::Webp) && job.source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("avif"))
            || matches!(format, Format::Avif) && job.source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("webp") || ext.eq_ignore_ascii_case("avif")) {
            decode_png(&job.source, &intermediate)?;
            intermediate.as_path()
        } else {
            job.source.as_path()
        };
        encode(input, &temporary, format, quality)?;
        if fs::metadata(&temporary)?.len() == 0 || probe(&temporary, false)? != dimensions {
            return Err(io::Error::other("output is empty or its dimensions changed"));
        }
        let new_size = fs::metadata(&temporary)?.len();
        if job.destination != job.source && job.destination.exists() {
            return Err(io::Error::other("destination appeared during conversion"));
        }
        fs::rename(&temporary, &job.destination)?;
        let removed = if delete_original && job.destination != job.source {
            fs::remove_file(&job.source)?;
            true
        } else {
            delete_original && job.destination == job.source
        };
        Ok((old_size, new_size, removed))
    })();
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(&intermediate);
    result
}

fn probe(path: &Path, count_frames: bool) -> io::Result<(u32, u32)> {
    let mut command = Command::new("ffprobe");
    command.args(["-v", "error"]);
    if count_frames { command.arg("-count_frames"); }
    command.args(["-select_streams", "v:0", "-show_entries",
        if count_frames { "stream=width,height,nb_read_frames" } else { "stream=width,height" },
        "-of", "default=noprint_wrappers=1"]);
    let output = command.arg(path).output()?;
    if !output.status.success() { return Err(io::Error::other("cannot read image dimensions")); }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut width = None;
    let mut height = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("width=") { width = value.parse().ok(); }
        if let Some(value) = line.strip_prefix("height=") { height = value.parse().ok(); }
        if let Some(value) = line.strip_prefix("nb_read_frames=") {
            if let Ok(count) = value.parse::<u32>() {
                if count > 1 { return Err(io::Error::other("animated images are not supported")); }
            }
        }
    }
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err(io::Error::other("cannot read image dimensions")),
    }
}

fn decode_png(source: &Path, output: &Path) -> io::Result<()> {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-noautorotate", "-i"])
        .arg(source).args(["-frames:v", "1", "-c:v", "png", "-y"]).arg(output);
    run_command(&mut command)
}

fn encode(source: &Path, output: &Path, format: Format, quality: u8) -> io::Result<()> {
    let level = ((quality as u16 * 9 + 50) / 100).to_string();
    let jpeg_quality = (2 + (100 - quality as u16) * 29 / 99).to_string();
    let quality = quality.to_string();
    let mut command = match format {
        Format::Jpg | Format::Png => {
            let mut command = Command::new("ffmpeg");
            command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-noautorotate", "-i"])
                .arg(source).args(["-frames:v", "1", "-map_metadata", "0"]);
            match format {
                Format::Jpg => { command.args(["-c:v", "mjpeg", "-q:v", &jpeg_quality]); }
                _ => { command.args(["-c:v", "png", "-compression_level", &level]); }
            }
            command.arg("-y").arg(output);
            command
        }
        Format::Webp => {
            let mut command = Command::new("cwebp");
            command.args(["-quiet", "-q", &quality, "-metadata", "all"])
                .arg(source).arg("-o").arg(output);
            command
        }
        Format::Avif => {
            let mut command = Command::new("avifenc");
            command.args(["--jobs", "2", "-q", &quality, "--qalpha", "100"])
                .arg(source).arg(output);
            command
        }
    };
    run_command(&mut command)
}

fn run_command(command: &mut Command) -> io::Result<()> {
    let output = command.output()?;
    if output.status.success() { return Ok(()); }
    let message = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::other(message.lines().last().unwrap_or("image encoder failed").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_savings_and_growth() {
        assert_eq!(space_summary(2048, 1024, true), "Estimated space saved: 1.00 KiB (50.0%)");
        assert_eq!(space_summary(1024, 2048, true), "Estimated extra space used: 1.00 KiB (100.0%)");
        assert_eq!(space_summary(2048, 1024, false), "Additional space used: 1.00 KiB (originals kept)");
    }
}
