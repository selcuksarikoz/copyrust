use std::io;
use std::path::Path;
use std::process::Command;

pub struct DiskInfo {
    pub filesystem: String,
    pub solid_state: bool,
    pub case_insensitive: bool,
}

// diskutil needs the mounted volume path, not an arbitrary target subfolder.
pub fn detect(path: &Path) -> io::Result<DiskInfo> {
    let df = Command::new("df").arg("-P").arg(path).output()?;
    if !df.status.success() {
        return Err(io::Error::other("df could not find the target volume"));
    }
    let df_text = String::from_utf8_lossy(&df.stdout);
    let mount = df_text.lines().nth(1)
        .map(|line| line.split_whitespace().skip(5).collect::<Vec<_>>().join(" "))
        .filter(|mount| !mount.is_empty())
        .ok_or_else(|| io::Error::other("df did not report a mount point"))?;
    let output = Command::new("diskutil").args(["info", "-plist"]).arg(mount).output()?;
    if !output.status.success() {
        return Err(io::Error::other("diskutil could not inspect target volume"));
    }
    let plist = String::from_utf8_lossy(&output.stdout);
    let filesystem = string_value(&plist, "FilesystemType")
        .ok_or_else(|| io::Error::other("diskutil did not report a filesystem"))?
        .to_string();
    let visible = string_value(&plist, "FilesystemUserVisibleName").unwrap_or("");
    let solid_state = bool_value(&plist, "SolidState")
        .ok_or_else(|| io::Error::other("diskutil did not report disk type"))?;
    if bool_value(&plist, "WritableVolume") == Some(false) {
        return Err(io::Error::other("target volume is read-only; copying requires a writable NTFS driver or another filesystem"));
    }
    let fs = filesystem.to_ascii_lowercase();
    let case_insensitive = fs.contains("exfat") || fs.contains("ntfs")
        || ((fs.contains("apfs") || fs.contains("hfs"))
            && !visible.to_ascii_lowercase().contains("case-sensitive"));
    Ok(DiskInfo { filesystem, solid_state, case_insensitive })
}

fn value_after_key<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("<key>{key}</key>");
    plist.split_once(&marker).map(|(_, rest)| rest.trim_start())
}

fn string_value<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    value_after_key(plist, key)?.strip_prefix("<string>")?.split_once("</string>").map(|(value, _)| value)
}

fn bool_value(plist: &str, key: &str) -> Option<bool> {
    let value = value_after_key(plist, key)?;
    if value.starts_with("<true/>") { Some(true) }
    else if value.starts_with("<false/>") { Some(false) }
    else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_diskutil_fields() {
        let plist = "<key>FilesystemType</key>\n<string>exfat</string>\n<key>SolidState</key>\n<false/>";
        assert_eq!(string_value(plist, "FilesystemType"), Some("exfat"));
        assert_eq!(bool_value(plist, "SolidState"), Some(false));
    }
}
