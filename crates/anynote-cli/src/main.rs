//! `anynote`: work with an AnyNote store from the terminal.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::Context;
use anynote_core::{folder, paths, Store};
use clap::{Parser, Subcommand};

/// `println!` that exits quietly when stdout is closed (e.g. piped into `head`).
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write;
        if writeln!(std::io::stdout(), $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

#[derive(Parser)]
#[command(name = "anynote", version, about = "AnyNote command-line tools")]
struct Cli {
    /// Store location (defaults to the desktop app's data folder).
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show notebooks, sections and pages as a tree.
    List,
    /// Import a OneNote .one section, .onetoc2 notebook, .onepkg package, or a folder of them.
    ImportOnenote { path: PathBuf },
    /// Import an AnyNote export or any folder of Markdown files.
    Import { folder: PathBuf },
    /// Export notebooks to the open folder format (Markdown + JSON sidecars).
    Export {
        out_dir: PathBuf,
        /// Only export these notebooks (by id); all notebooks when omitted.
        #[arg(long = "notebook", value_name = "ID")]
        notebooks: Vec<String>,
    },
    /// Full-text search.
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Tasks across all notebooks (open ones unless --all).
    Tasks {
        #[arg(long)]
        all: bool,
    },
    /// Tags and how many pages use them.
    Tags,
    /// Dump the parsed object tree of a OneNote file as JSON.
    OnenoteDump {
        file: PathBuf,
        /// Write to a file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Rebuild the search, link, task and tag index.
    Reindex,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Command::OnenoteDump { file, out } = &cli.command {
        // Needs no store.
        let json = serde_json::to_string_pretty(&anynote_onenote::dump_json(file)?)?;
        match out {
            Some(path) => {
                std::fs::write(path, json + "\n").with_context(|| format!("cannot write {}", path.display()))?
            }
            None => out!("{json}"),
        }
        return Ok(());
    }

    let data_dir = cli.data_dir.unwrap_or_else(paths::default_data_dir);
    let mut store = Store::open(&data_dir).with_context(|| format!("cannot open store at {}", data_dir.display()))?;
    let style = Style::new();

    match cli.command {
        Command::List => list(&store, &style)?,
        Command::ImportOnenote { path } => {
            let report = anynote_onenote::import(&mut store, &path)?;
            out!(
                "Imported {} notebook(s), {} section(s), {} page(s), {} asset(s), {} ink stroke(s).",
                report.notebook_ids.len(),
                report.sections,
                report.pages,
                report.assets,
                report.ink_strokes
            );
            print_warnings(&report.warnings, &style);
        }
        Command::Import { folder: dir } => {
            let summary = folder::import(&mut store, &dir)?;
            out!(
                "Imported {} notebook(s), {} section(s), {} page(s), {} asset(s).",
                summary.notebook_ids.len(),
                summary.sections,
                summary.pages,
                summary.assets
            );
            print_warnings(&summary.warnings, &style);
        }
        Command::Export { out_dir, notebooks } => {
            let summary = folder::export(&mut store, &out_dir, &notebooks)?;
            out!(
                "Exported {} notebook(s), {} section(s), {} page(s), {} asset(s) to {}.",
                summary.notebooks,
                summary.sections,
                summary.pages,
                summary.assets,
                out_dir.display()
            );
        }
        Command::Search { query, limit } => {
            let hits = store.search(&query.join(" "), limit)?;
            if hits.is_empty() {
                out!("No matches.");
            }
            for hit in hits {
                let p = &hit.page;
                out!(
                    "{} {}",
                    style.bold(title_or_untitled(&p.title)),
                    style.dim(&format!("{} › {}  {}", p.notebook_title, p.section_title, p.id))
                );
                let snippet = hit
                    .snippet
                    .replace('\n', " ")
                    .replace('\u{1}', style.mark_start())
                    .replace('\u{2}', style.mark_end());
                out!("    {snippet}");
            }
        }
        Command::Tasks { all } => {
            let tasks = store.tasks(all)?;
            if tasks.is_empty() {
                out!("No tasks.");
            }
            let mut last_page = String::new();
            for t in tasks {
                if t.page.id != last_page {
                    out!(
                        "{} {}",
                        style.bold(title_or_untitled(&t.page.title)),
                        style.dim(&format!("{} › {}", t.page.notebook_title, t.page.section_title))
                    );
                    last_page = t.page.id.clone();
                }
                out!("  [{}] {}", if t.checked { "x" } else { " " }, t.text);
            }
        }
        Command::Tags => {
            for t in store.tags()? {
                out!("#{} ({})", t.tag, t.count);
            }
        }
        Command::Reindex => {
            let n = store.reindex_all()?;
            out!("Reindexed {n} page(s).");
        }
        Command::OnenoteDump { .. } => unreachable!("handled above"),
    }
    Ok(())
}

fn list(store: &Store, style: &Style) -> anyhow::Result<()> {
    let notebooks = store.list_notebooks()?;
    if notebooks.is_empty() {
        out!("No notebooks yet. Try `anynote import-onenote <file>` or `anynote import <folder>`.");
    }
    for nb in notebooks {
        out!("{} {}", style.bold(&nb.title), style.dim(&nb.id));
        let sections = store.list_sections(&nb.id)?;
        print_sections(store, style, &sections, None, 1)?;
    }
    Ok(())
}

fn print_sections(
    store: &Store,
    style: &Style,
    sections: &[anynote_core::store::SectionInfo],
    parent: Option<&str>,
    depth: usize,
) -> anyhow::Result<()> {
    for sec in sections.iter().filter(|s| s.parent_id.as_deref() == parent) {
        out!("{}§ {}", "  ".repeat(depth), sec.title);
        let pages = store.list_pages(&sec.id)?;
        print_pages(style, &pages, None, depth + 1);
        print_sections(store, style, sections, Some(&sec.id), depth + 1)?;
    }
    Ok(())
}

fn print_pages(style: &Style, pages: &[anynote_core::store::PageInfo], parent: Option<&str>, depth: usize) {
    for page in pages.iter().filter(|p| p.parent_id.as_deref() == parent) {
        out!("{}- {} {}", "  ".repeat(depth), title_or_untitled(&page.title), style.dim(&page.id));
        print_pages(style, pages, Some(&page.id), depth + 1);
    }
}

fn title_or_untitled(t: &str) -> &str {
    if t.is_empty() {
        "Untitled"
    } else {
        t
    }
}

fn print_warnings(warnings: &[String], style: &Style) {
    if warnings.is_empty() {
        return;
    }
    eprintln!("{} warning(s):", warnings.len());
    for w in warnings {
        eprintln!("  {} {w}", style.dim("•"));
    }
}

/// ANSI styling, only when writing to a terminal.
struct Style {
    color: bool,
}

impl Style {
    fn new() -> Self {
        Style { color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() }
    }

    fn bold(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn dim(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn mark_start(&self) -> &'static str {
        if self.color {
            "\x1b[1;33m"
        } else {
            "["
        }
    }

    fn mark_end(&self) -> &'static str {
        if self.color {
            "\x1b[0m"
        } else {
            "]"
        }
    }
}
