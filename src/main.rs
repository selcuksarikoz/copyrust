mod disk;
mod files;
mod prompt;

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use files::{common_root, ConflictAction, Plan, PostCopyAction};
use prompt::{dir_path, menu, select_extensions, source_folders};

// Parallel workers to use per target disk type: HDDs hate concurrent random I/O.
const HDD_WORKERS: usize = 1;
const SSD_WORKERS: usize = 4;

// Bundle of the user's run choices, threaded through the copy phase.
struct CopyConfig {
    flatten: bool,
    conflict: ConflictAction,
    post: PostCopyAction,
    workers: usize,
    case_insensitive: bool,
}

#[derive(Default, Clone, Copy)]
struct Stats {
    copied: u32,
    renamed: u32,
    skipped: u32,
    deleted: u32,
    failed: u32,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    println!("=== copyrust ===\n");

    // 1) Collect files from the source folders, then select them by extension.
    let input = source_folders()?;
    let mut all = input.files;
    all.sort_unstable();
    all.dedup();
    if all.is_empty() {
        println!("No files found in source.");
        return Ok(());
    }
    let source = if input.dirs.len() == 1 {
        input.dirs[0].clone()
    } else {
        common_root(&all)
    };
    let picked = select_extensions(&all)?;
    let chosen: Vec<PathBuf> = picked.iter().map(|&i| all[i].clone()).collect();
    print!("\n{} file(s) selected.\n\n", chosen.len());

    // 2) Target folder, disk type, and layout (created if missing).
    let target = dir_path("Target folder (created if missing)", false)?;
    if input.dirs.iter().any(|dir| target.starts_with(dir) || dir.starts_with(&target)) {
        return Err(io::Error::other("source and target folders must not overlap"));
    }
    std::fs::create_dir_all(&target)?;
    let disk = disk::detect(&target)?;
    let workers = if disk.solid_state { SSD_WORKERS } else { HDD_WORKERS };
    println!("Target disk: {} / {} ({} worker(s)).\n",
        disk.filesystem, if disk.solid_state { "SSD" } else { "HDD" }, workers);
    let flatten = menu(
        "Keep subfolder structure inside the target?",
        &["Yes, preserve structure", "No, flatten into target root"],
        0,
    )? == 1;

    // 3) Conflict action for files that already exist in the target.
    let conflict = menu(
        "When a file already exists in the target:",
        &["Overwrite", "Skip", "Rename (keep both)"],
        2,
    )?;
    let conflict = [ConflictAction::Overwrite, ConflictAction::Skip, ConflictAction::Rename][conflict];

    // 4) Post-copy action; deleting the source is the default.
    let post = menu(
        "After a successful copy, delete the source file?",
        &["Yes, delete from source", "No, keep in source"],
        0,
    )?;
    let post = [PostCopyAction::Delete, PostCopyAction::Keep][post];

    let config = CopyConfig { flatten, conflict, post, workers, case_insensitive: disk.case_insensitive };

    // One indexing pass over the target keeps per-file checks off the slow disk entirely.
    println!("\nIndexing target...");
    let mut index = files::index_target(&target, config.flatten, config.case_insensitive)?;
    println!("Target holds {} existing item(s).", index.len());

    let stats = copy_all(&chosen, &source, &target, &config, &mut index)?;

    println!(
        "\nDone. copied: {}, renamed: {}, skipped: {}, source deleted: {}, failed: {}",
        stats.copied, stats.renamed, stats.skipped, stats.deleted, stats.failed
    );
    Ok(())
}

// Plans every job up front (no per-file stats), then runs plans on a worker pool
// while drawing a progress bar. `workers == 1` degrades to plain sequential copy.
fn copy_all(
    chosen: &[PathBuf],
    source: &Path,
    target: &Path,
    config: &CopyConfig,
    index: &mut HashMap<String, PathBuf>,
) -> io::Result<Stats> {
    let CopyConfig { flatten, conflict, post, workers, case_insensitive } = *config;
    let plans = files::build_plans(chosen, source, target, flatten, conflict, case_insensitive, index)?;
    let total = plans.len();
    let mut destinations = HashSet::new();
    let repeated_destination = plans.iter().any(|plan| match plan {
        Plan::Copy { dest, .. } => !destinations.insert(dest.clone()),
        Plan::Skip => false,
    });
    let workers = if repeated_destination { 1 } else { workers };
    println!("Copying with {workers} worker(s)...\n");

    let (job_tx, job_rx) = mpsc::channel::<(usize, Plan)>();
    let (res_tx, res_rx) = mpsc::channel::<(usize, io::Result<()>)>();

    // Workers pull plans from one shared queue (std channels are single-consumer).
    let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
    for _ in 0..workers {
        let job_rx = std::sync::Arc::clone(&job_rx);
        let res_tx = res_tx.clone();
        thread::spawn(move || loop {
            let job = { job_rx.lock().unwrap().recv() };
            let Ok((i, plan)) = job else { break };
            let _ = res_tx.send((i, files::execute(&plan)));
        });
    }
    for (i, plan) in plans.iter().enumerate() {
        job_tx
            .send((i, plan.clone()))
            .map_err(|_| io::Error::other("worker pool died"))?;
    }
    drop(job_tx);
    drop(res_tx);

    let mut stats = Stats::default();
    for (done, (i, result)) in res_rx.iter().take(total).enumerate() {
        let Plan::Copy { src, renamed, .. } = &plans[i] else {
            stats.skipped += 1;
            draw_progress(done + 1, total, stats.failed);
            continue;
        };
        match result {
            Ok(()) => {
                if *renamed { stats.renamed += 1 } else { stats.copied += 1 }
                // Default flow removes the source after a verified successful copy.
                if post == PostCopyAction::Delete {
                    match std::fs::remove_file(src) {
                        Ok(()) => stats.deleted += 1,
                        Err(e) => {
                            stats.failed += 1;
                            eprintln!("\r\x1b[KFAILED delete {}: {e}", src.display());
                        }
                    }
                }
            }
            Err(e) => {
                stats.failed += 1;
                eprintln!("\r\x1b[KFAILED copy {}: {e}", src.display());
            }
        }
        draw_progress(done + 1, total, stats.failed);
    }

    println!();
    Ok(stats)
}

// Renders a single-line percentage progress bar.
fn draw_progress(done: usize, total: usize, failed: u32) {
    if total == 0 {
        return;
    }
    let width = 30;
    let filled = done * width / total;
    let pct = ((done * 100) / total).min(100);
    print!(
        "\r[{}{}] {:>3}% ({done}/{total}) failed: {failed} ",
        "=".repeat(filled),
        " ".repeat(width - filled),
        pct
    );
    let _ = io::stdout().flush();
}
