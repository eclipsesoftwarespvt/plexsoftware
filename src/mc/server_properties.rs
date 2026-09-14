use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const FILE: &str = "server.properties";

const EOL: &str = "\r\n";

pub fn rewrite_dir<P: AsRef<Path>>(dir: P, changes: HashMap<&str, String>) {
    if changes.is_empty() {
        return;
    }

    if !dir.as_ref().is_dir() {
        warn!(target: "plexpaper",
            "Not rewriting {} file, configured server directory doesn't exist: {}",
            FILE,
            dir.as_ref().to_str().unwrap_or("?")
        );
        return;
    }

    rewrite_file(dir.as_ref().join(FILE), changes)
}

pub fn rewrite_file<P: AsRef<Path>>(file: P, changes: HashMap<&str, String>) {
    if changes.is_empty() {
        return;
    }

    if !file.as_ref().is_file() {
        warn!(target: "plexpaper",
            "Not writing {} file, not found at: {}",
            FILE,
            file.as_ref().to_str().unwrap_or("?"),
        );
        return;
    }

    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(err) => {
            error!(target: "plexpaper",
                "Failed to rewrite {} file, could not load: {}",
                FILE,
                err,
            );
            return;
        }
    };

    let contents = match rewrite_contents(contents, changes) {
        Some(contents) => contents,
        None => {
            debug!(target: "plexpaper",
                "Not rewriting {} file, no changes to apply",
                FILE,
            );
            return;
        }
    };

    match fs::write(file, contents) {
        Ok(_) => {
            debug!(target: "plexpaper",
                "Rewritten {} file with updated values",
                FILE,
            );
        }
        Err(err) => {
            error!(target: "plexpaper",
                "Failed to rewrite {} file, could not save changes: {}",
                FILE,
                err,
            );
        }
    };
}

fn rewrite_contents(contents: String, mut changes: HashMap<&str, String>) -> Option<String> {
    if changes.is_empty() {
        return None;
    }

    let mut changed = false;

    let mut new_contents: String = contents
        .lines()
        .map(|line| {
            let mut line = line.to_owned();

            let trim = line.trim();
            if trim.starts_with('#') || trim.is_empty() {
                return line;
            }

            let (key, value) = match line.split_once('=') {
                Some(result) => result,
                None => return line,
            };

            if let Some((_, new)) = changes.remove_entry(key.trim().to_lowercase().as_str()) {
                if value != new {
                    line = format!("{key}={new}");
                    changed = true;
                }
            }

            line
        })
        .collect::<Vec<_>>()
        .join(EOL);

    for (key, value) in changes {
        new_contents += &format!("{EOL}{key}={value}");
        changed = true;
    }

    if changed {
        Some(new_contents)
    } else {
        None
    }
}

pub fn read_property<P: AsRef<Path>>(file: P, property: &str) -> Option<String> {

    if !file.as_ref().is_file() {
        warn!(target: "plexpaper",
            "Failed to read property from {} file, it does not exist",
            FILE,
        );
        return None;
    }

    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(err) => {
            error!(target: "plexpaper",
                "Failed to read property from {} file, could not load: {}",
                FILE,
                err,
            );
            return None;
        }
    };

    contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(p, _)| p.trim().to_lowercase() == property.to_lowercase())
        .map(|(_, v)| v.trim().to_string())
}
