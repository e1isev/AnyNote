//! Locating and parsing OneNote files.

use std::path::Path;

use anyhow::{bail, Context};
use onenote_parser::notebook::Notebook;
use onenote_parser::section::Section;
use onenote_parser::Parser;
use typed_path::TypedPath;

pub enum Parsed {
    Notebook { title: String, notebook: Notebook },
    Section(Section),
}

/// Lower-case extension; also for dot-files such as `.onetoc2`.
pub fn ext(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Run the parser with `path` converted to the `TypedPath` it expects.
fn with_typed<T>(path: &Path, f: impl FnOnce(TypedPath) -> T) -> anyhow::Result<T> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(f(TypedPath::new(path.as_os_str().as_bytes(), typed_path::PathType::Unix)))
    }
    #[cfg(not(unix))]
    {
        let s = path.to_str().with_context(|| format!("path is not valid UTF-8: {}", path.display()))?;
        Ok(f(TypedPath::derive(s)))
    }
}

/// Parse one `.one`, `.onetoc2` or `.onepkg` file.
pub fn parse(path: &Path) -> anyhow::Result<Parsed> {
    let parser = Parser::new();
    let display = path.display().to_string();
    match ext(path).as_str() {
        "onetoc2" => {
            let notebook = with_typed(path, |p| parser.parse_notebook(p))?
                .with_context(|| format!("failed to parse notebook {display}"))?;
            Ok(Parsed::Notebook { title: notebook_title(path), notebook })
        }
        "onepkg" => {
            let notebook = with_typed(path, |p| parser.parse_package(p))?
                .with_context(|| format!("failed to parse package {display}"))?;
            let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Notebook").to_string();
            Ok(Parsed::Notebook { title, notebook })
        }
        "one" => {
            let section = with_typed(path, |p| parser.parse_section(p))?
                .with_context(|| format!("failed to parse section {display}"))?;
            Ok(Parsed::Section(section))
        }
        other => bail!("{display}: not a OneNote file (.one, .onetoc2 or .onepkg), got .{other}"),
    }
}

/// A notebook's name is the folder holding its table of contents.
fn notebook_title(toc: &Path) -> String {
    toc.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .or_else(|| toc.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .unwrap_or_else(|| "Notebook".into())
}
