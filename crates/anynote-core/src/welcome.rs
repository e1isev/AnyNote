//! The notebook a new user starts with.

use crate::store::{Placement, Store};
use crate::{markdown, Result};

/// On first run (no notebooks and nothing in the trash), create a notebook
/// with a page that shows what pages can hold.
pub fn seed_welcome(store: &mut Store) -> Result<()> {
    if !store.list_notebooks()?.is_empty() || !store.list_trash()?.is_empty() {
        return Ok(());
    }
    let nb = store.create_notebook("My notebook", Some("#7c5cff"))?;
    let sec = store.create_section(&nb.id, None, "Getting started")?;
    let doc = markdown::from_markdown(include_str!("welcome.md"), &mut |_| None);
    store.create_page_with_content(
        &sec.id,
        None,
        "Welcome to AnyNote",
        Placement::End,
        &doc,
        &Default::default(),
        None,
    )?;
    store.create_section(&nb.id, None, "Ideas")?;
    Ok(())
}
