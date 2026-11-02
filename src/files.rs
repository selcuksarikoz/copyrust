use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// What to do when a file already exists in the target.
#[derive(Clone, Copy)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Rename,
}

// What to do with the source file after a successful copy.
#[derive(Clone, Copy, PartialEq)]
pub enum PostCopyAction {
    Delete,
    Keep,
}

// Recursively collects all regular files under `dir`, sorted for stable listing.
pub fn collect_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

// Slash-separated path relative to `root` (just the file name when `flatten` is set).
pub fn rel_key(path: &Path, root: &Path, flatten: bool) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    match flatten {
        true => rel.file_name().map(|n| n.to_string_lossy().into_owned()),
        false => Some(rel.to_string_lossy().replace('\\', "/")),
    }
}

// Indexes the target once (one walk) so per-file existence checks never touch the slow disk again.
pub fn index_target(target: &Path, flatten: bool, case_insensitive: bool) -> io::Result<HashMap<String, PathBuf>> {
    let mut index = HashMap::new();
    let mut stack = vec![target.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() && !flatten {
                stack.push(path.clone());
            }
            if (kind.is_file() || kind.is_dir()) && let Some(key) = rel_key(&path, target, flatten) {
                index.insert(normalize_key(&key, case_insensitive), path);
            }
        }
    }
    Ok(index)
}

fn normalize_key(key: &str, case_insensitive: bool) -> String {
    if case_insensitive { key.to_lowercase() } else { key.to_owned() }
}

// What a worker has to do for one selected file.
#[derive(Clone)]
pub enum Plan {
    Skip,
    Copy { src: PathBuf, dest: PathBuf, renamed: bool },
}

// Decides the action for every selected file in one pass; planned destinations are
// registered in `index` so later duplicates and real target collisions agree.
pub fn build_plans(
    selected: &[PathBuf],
    source: &Path,
    target: &Path,
    flatten: bool,
    action: ConflictAction,
    case_insensitive: bool,
    index: &mut HashMap<String, PathBuf>,
) -> io::Result<Vec<Plan>> {
    let mut plans = Vec::with_capacity(selected.len());
    for src in selected {
        let key = rel_key(src, source, flatten)
            .ok_or_else(|| io::Error::other(format!("cannot relativize {}", src.display())))?;
        let normalized = normalize_key(&key, case_insensitive);
        plans.push(if let Some(existing) = index.get(&normalized) {
            match action {
                ConflictAction::Skip => Plan::Skip,
                ConflictAction::Overwrite => Plan::Copy {
                    src: src.clone(),
                    dest: existing.clone(),
                    renamed: false,
                },
                ConflictAction::Rename => {
                    let dest = dest_path(target, &key);
                    let dest = unique_dest(&dest, target, flatten, case_insensitive, index)?;
                    Plan::Copy { src: src.clone(), dest, renamed: true }
                }
            }
        } else {
            let dest = dest_path(target, &key);
            index.insert(normalized, dest.clone());
            Plan::Copy { src: src.clone(), dest, renamed: false }
        });
    }
    Ok(plans)
}

pub fn dest_path(target: &Path, key: &str) -> PathBuf {
    target.join(key.replace('/', std::path::MAIN_SEPARATOR_STR))
}

// Longest ancestor shared by every listed path, used as the rel-key base for loose files.
pub fn common_root(paths: &[PathBuf]) -> PathBuf {
    let mut root: Vec<std::ffi::OsString> =
        paths[0].components().map(|c| c.as_os_str().to_owned()).collect();
    for p in paths {
        let comps = p.components();
        let shared = root
            .iter()
            .zip(comps)
            .take_while(|(a, b)| a == &&b.as_os_str().to_owned())
            .count();
        root.truncate(shared);
    }
    root.into_iter().collect()
}

// Finds `name (1).ext`, `name (2).ext`, ... not present in the index, and registers it.
fn unique_dest(
    dest: &Path,
    target: &Path,
    flatten: bool,
    case_insensitive: bool,
    index: &mut HashMap<String, PathBuf>,
) -> io::Result<PathBuf> {
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = dest.extension().and_then(|s| s.to_str());
    for n in 1u32.. {
        let candidate = match ext {
            Some(e) => dest.with_file_name(format!("{stem} ({n}).{e}")),
            None => dest.with_file_name(format!("{stem} ({n})")),
        };
        let key = rel_key(&candidate, target, flatten).unwrap_or_default();
        let normalized = normalize_key(&key, case_insensitive);
        if !index.contains_key(&normalized) {
            index.insert(normalized, candidate.clone());
            return Ok(candidate);
        }
    }
    unreachable!()
}

// Performs the actual copy for one plan.
pub fn execute(plan: &Plan) -> io::Result<()> {
    let Plan::Copy { src, dest, .. } = plan else { return Ok(()) };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_conflicts_follow_selected_action() {
        let source = Path::new("/source");
        let target = Path::new("/target");
        let selected = vec![source.join("photo.jpg")];
        let existing = target.join("Photo.JPG");
        let base = HashMap::from([("photo.jpg".to_string(), existing.clone())]);

        let mut index = base.clone();
        let plans = build_plans(&selected, source, target, false, ConflictAction::Skip, true, &mut index).unwrap();
        assert!(matches!(plans[0], Plan::Skip));

        let mut index = base.clone();
        let plans = build_plans(&selected, source, target, false, ConflictAction::Overwrite, true, &mut index).unwrap();
        assert!(matches!(&plans[0], Plan::Copy { dest, .. } if *dest == existing));

        let mut index = base;
        let plans = build_plans(&selected, source, target, false, ConflictAction::Rename, true, &mut index).unwrap();
        assert!(matches!(&plans[0], Plan::Copy { dest, renamed: true, .. } if *dest == target.join("photo (1).jpg")));
    }
}
