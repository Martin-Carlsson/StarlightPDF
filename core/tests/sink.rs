//! A file sink's path, read when the sink is made; alone in its binary, as it changes the
//! working directory.

use core::error::Error;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use starlightpdf::arrange::InsertBlankPageOptions;
use starlightpdf::document::{CreateOptions, Document, SaveOptions};
use starlightpdf::output::{FileSinkOptions, Sink};

/// A relative path to a file not there yet names it in the working directory the sink is made
/// in, not the one it is saved in.
#[test]
fn a_relative_path_is_read_when_the_sink_is_made() -> Result<(), Box<dyn Error>> {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sink");
    let (made, saved) = (base.join("made"), base.join("saved"));
    for dir in [&made, &saved] {
        match fs::remove_dir_all(dir) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
            Ok(()) | Err(_) => {}
        }
        fs::create_dir_all(dir)?;
    }
    env::set_current_dir(&made)?;
    let mut options = FileSinkOptions::default();
    options.file = PathBuf::from("out.pdf");
    let sink = Sink::from_file(&options)?;
    env::set_current_dir(&saved)?;
    let document = Document::create(None, &CreateOptions::default())?;
    drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    drop(document.save(&sink, None, &SaveOptions::default())?);
    if !made.join("out.pdf").is_file() || saved.join("out.pdf").exists() {
        return Err("the file was not written where the sink was made".into());
    }
    Ok(())
}
