use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// Reads one trimmed line from stdin; a closed pipe (EOF/Ctrl-C) is a hard error.
pub fn read_line() -> io::Result<String> {
    let mut buf = String::new();
    io::stdout().flush()?;
    let n = io::stdin().read_line(&mut buf)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "input closed (EOF)"));
    }
    Ok(buf.trim().to_string())
}

// Shows a numbered menu, returns the index of the chosen option. Empty input picks `default`.
pub fn menu(title: &str, options: &[&str], default: usize) -> io::Result<usize> {
    println!("{title}");
    for (i, opt) in options.iter().enumerate() {
        println!("  {}. {opt}", i + 1);
    }
    loop {
        print!("Choose [1-{}, default: {}]: ", options.len(), default + 1);
        let input = read_line()?;
        if input.is_empty() {
            println!();
            return Ok(default);
        }
        match input.parse::<usize>() {
            Ok(n) if (1..=options.len()).contains(&n) => {
                println!();
                return Ok(n - 1);
            }
            _ => println!("Invalid choice, try again."),
        }
    }
}

// Result of the source prompt: folders given as roots plus their files.
pub struct SourceInput {
    pub dirs: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

// Prompts for one or more source folders.
pub fn source_folders() -> io::Result<SourceInput> {
    loop {
        print!("Source folders (paste/drop folder paths): ");
        let input = read_line()?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut invalid = false;
        for token in source_tokens(&input) {
            match fs::canonicalize(&token) {
                Ok(p) if p.is_dir() => {
                    files.extend(crate::files::collect_files(&p)?);
                    dirs.push(p);
                }
                _ => {
                    println!("  not a folder: {token}");
                    invalid = true;
                }
            }
        }
        if invalid || dirs.is_empty() {
            println!("Enter one or more folders, then try again.");
            continue;
        }
        files.sort_unstable();
        files.dedup();
        return Ok(SourceInput { dirs, files });
    }
}

// Finder's terminal drop uses spaces between paths and backslashes for spaces in names.
fn source_tokens(input: &str) -> Vec<String> {
    fn unquote(input: &str, split_spaces: bool) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut token = String::new();
        let mut quote = None;
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => token.push(chars.next().unwrap_or('\\')),
                '\'' | '"' if quote == Some(c) => quote = None,
                '\'' | '"' if quote.is_none() => quote = Some(c),
                c if split_spaces && c.is_whitespace() && quote.is_none() => {
                    if !token.is_empty() {
                        tokens.push(std::mem::take(&mut token));
                    }
                }
                _ => token.push(c),
            }
        }
        if !token.is_empty() {
            tokens.push(token);
        }
        tokens
    }

    if input.contains(',') {
        return input.split(',').flat_map(|part| unquote(part.trim(), false)).collect();
    }
    let split = unquote(input.trim(), true);
    if split.len() > 1 && split.iter().all(|path| fs::canonicalize(path).is_ok()) {
        split
    } else {
        unquote(input.trim(), false)
    }
}

// Prompts for a directory path; `must_exist=false` accepts paths to be created.
pub fn dir_path(question: &str, must_exist: bool) -> io::Result<PathBuf> {
    loop {
        print!("{question}: ");
        let input = read_line()?;
        // Strip surrounding quotes so paths can be pasted from Finder "Copy as Pathname".
        let cleaned = input.trim_matches(|c| c == '"' || c == '\'').to_string();
        let path = PathBuf::from(cleaned);
        // Canonicalize when possible (existing dir, or dir whose parent exists);
        // otherwise keep the raw path and let create_dir_all handle it later.
        let resolved = match fs::canonicalize(&path) {
            Ok(real) => real,
            Err(_) if !must_exist => match fs::canonicalize(path.parent().unwrap_or(Path::new("."))) {
                Ok(parent) => parent.join(path.file_name().unwrap_or_default()),
                Err(_) => path.clone(),
            },
            Err(_) => {
                println!("Not a valid directory: {input}");
                continue;
            }
        };
        if !must_exist || resolved.is_dir() {
            println!();
            return Ok(resolved);
        }
        println!("Not a valid directory: {input}");
    }
}

// Parses numbers/ranges like "1,3-5"; empty or "all" means everything.
fn parse_indices(token: &str, total: usize) -> Option<Vec<usize>> {
    let mut picked = Vec::new();
    for part in token.split(',') {
        let range = part.trim().split_once('-');
        let indices = match range {
            Some((a, b)) => match (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                (Ok(a), Ok(b)) if a >= 1 && b >= a && b <= total => Some((a - 1..=b - 1).collect()),
                _ => None,
            },
            None => match part.trim().parse::<usize>() {
                Ok(n) if (1..=total).contains(&n) => Some(vec![n - 1]),
                _ => None,
            },
        };
        picked.append(&mut indices?);
    }
    Some(picked)
}

// Lists the extensions present in a folder and selects every file in the chosen groups.
pub fn select_extensions(all: &[PathBuf]) -> io::Result<Vec<usize>> {
    let mut groups: BTreeMap<Option<String>, Vec<usize>> = BTreeMap::new();
    for (i, path) in all.iter().enumerate() {
        let ext = path.extension().filter(|ext| !ext.is_empty())
            .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
        groups.entry(ext).or_default().push(i);
    }
    let groups: Vec<_> = groups.into_iter().collect();
    println!("{} file(s) in source. Available extensions:", all.len());
    for (i, (ext, files)) in groups.iter().enumerate() {
        let label = ext.as_ref().map(|ext| format!(".{ext}")).unwrap_or_else(|| "(no extension)".into());
        println!("  {}. {} — {} file(s)", i + 1, label, files.len());
    }

    loop {
        print!("\nChoose extensions (e.g. 1,3-5; empty = all): ");
        let input = read_line()?;
        if input.is_empty() || input.eq_ignore_ascii_case("all") {
            return Ok((0..all.len()).collect());
        }
        if let Some(picked) = parse_indices(&input, groups.len()) {
            let mut files: Vec<usize> = picked.into_iter()
                .flat_map(|i| groups[i].1.iter().copied())
                .collect();
            files.sort_unstable();
            files.dedup();
            return Ok(files);
        }
        println!("Invalid choice, enter listed numbers separated by commas or a range.");
    }
}

