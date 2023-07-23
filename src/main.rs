use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader; use std::path::PathBuf;

use chrono::{Datelike, Timelike};
use clap::Parser;
use exif::{DateTime, In, Reader, Value, Tag};
use walkdir::WalkDir;

/// Copy all files from a directory tree into another, using names that match how Dropbox Camera
/// Uploads would rename them (additionally split up by year).
///
/// Date and time of files is taken from file metadata (EXIF tags) if possible, or file
/// modification otherwise.
#[derive(Debug, Parser)]
struct Args {
    /// Path to copy files from. This tree is walked recursively.
    #[arg(long)]
    src: PathBuf,

    /// Path to copy the files to. A subdirectory under this will be added for each year.
    #[arg(long)]
    dst: PathBuf,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    println!("{args:#?}");

    for entry in WalkDir::new(&args.src) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("failed to open file {path:?}: {e}");
                continue;
            }
        };
        let maybe_datetime = match path.extension().and_then(OsStr::to_str) {
            Some("jpg") => {
                Reader::new()
                    .read_from_container(&mut BufReader::new(&file))
                    .map_err(|e| {
                        eprintln!("failed to read exif from {path:?}: {e}");
                        e
                    })
                    .ok()
                    .and_then(|exif| {
                        exif.get_field(Tag::DateTimeOriginal, In::PRIMARY)
                            .and_then(|field| match field.value {
                                Value::Ascii(ref vec) if !vec.is_empty() => {
                                    if let Ok(datetime) = DateTime::from_ascii(&vec[0]) {
                                        Some(datetime)
                                    } else {
                                        eprintln!("{path:?}: unable to parse exif datetime {vec:?}");
                                        None
                                    }
                                }
                                _ => {
                                    eprintln!("{path:?}: exif datetime is not ascii: {field:?}");
                                    None
                                }
                            })
                    })
            }
            _ => None,
        };

        // temporary hax
        /*
        if maybe_datetime.is_some() {
            continue;
        }*/

        let datetime = maybe_datetime.unwrap_or_else(|| {
            let meta = file.metadata().unwrap();
            let chr: chrono::DateTime<chrono::Local> = meta.modified().unwrap().into();
            macro_rules! cast {
                ($n:expr) => {
                    $n.try_into().unwrap_or_else(|e| panic!("{} ({}): {}", stringify!($n), $n, e))
                }
            }
            DateTime {
                year: cast!(chr.year()),
                month: cast!(chr.month()),
                day: cast!(chr.day()),
                hour: cast!(chr.hour()),
                minute: cast!(chr.minute()),
                second: cast!(chr.second()),
                nanosecond: None,
                offset: None,
            }
        });

        let filename = |n: usize| {
            let mut s = format!("{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
                datetime.year,
                datetime.month,
                datetime.day,
                datetime.hour,
                datetime.minute,
                datetime.second);
            if n > 0 {
                s += &n.to_string();
            }
            if let Some(ext) = path.extension().and_then(OsStr::to_str) {
                s.push('.');
                s += ext;
            }
            s
        };

        let mut new_path = args.dst
            .join(datetime.year.to_string());

        if !new_path.exists() { std::fs::create_dir_all(&new_path).unwrap(); }

        new_path.push(filename(0));

        let mut n = 1;
        while new_path.exists() {
            new_path.set_file_name(filename(n));
            n += 1;
        }

        if let Err(e) = std::fs::copy(path, &new_path) {
            eprintln!("failed to copy {path:?} to {new_path:?}: {e}");
            continue;
        }
    }

    Ok(())
}
