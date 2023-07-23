use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use chrono::{Datelike, Timelike};
use clap::Parser;
use rexif::{ExifTag, TagValue};
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

    /// Don't actually copy, just display what would be copied.
    #[arg(long)]
    dry_run: bool,
}

struct DateTime {
    year: u32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

fn exif_datetime(path: &Path) -> anyhow::Result<DateTime> {
    let exif = rexif::parse_file(path)
        .context("failed to read EXIF")?;

    let field = exif.entries
        .into_iter()
        .find(|entry| entry.tag == ExifTag::DateTimeOriginal)
        .ok_or_else(|| anyhow!("no DateTimeOriginal EXIF tag found"))?;

    let value = match field.value {
        TagValue::Ascii(s) => s,
        _ => bail!("DateTimeOriginal EXIF tag has non-ASCII value: {:?}", field.value),
    };

    let dt = parse_exif_datetime(value.as_bytes())
        .with_context(|| format!("unable to parse EXIF DateTime {value:?}"))?;

    Ok(dt)
}

fn atou<T: num::CheckedAdd + num::CheckedMul + num::FromPrimitive + num::Zero>(bytes: &[u8]) -> anyhow::Result<T> {
    let mut n = T::zero();
    let ten = T::from_u8(10).unwrap();
    for b in bytes {
        if !b.is_ascii_digit() {
            bail!("non-ascii digit {b:#x}");
        }
        let digit = T::from_u8(u8::from(b - b'0')).unwrap();
        n = n.checked_mul(&ten)
            .and_then(|n| n.checked_add(&digit))
            .ok_or_else(|| anyhow!("overflow"))?;
    }
    Ok(n)
}

fn parse_exif_datetime(data: &[u8]) -> anyhow::Result<DateTime> {
    // cribbed from exif-rs
    if data == b"    :  :     :  :  " || data == b"                   " {
        bail!("EXIF DateTime is blank");
    } else if data.len() < 19 {
        bail!("EXIF DateTime too short");
    } else if !(data[4] == b':' && data[7] == b':' && data[10] == b' ' &&
                data[13] == b':' && data[16] == b':') {
        bail!("Invalid EXIF DateTime delimiter");
    }
    Ok(DateTime {
        year: atou(&data[0..4])?,
        month: atou(&data[5..7])?,
        day: atou(&data[8..10])?,
        hour: atou(&data[11..13])?,
        minute: atou(&data[14..16])?,
        second: atou(&data[17..19])?,
    })
}

fn mtime_datetime(file: &File) -> DateTime {
    let meta = file.metadata().expect("should be able to read metadata from open file");
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
    }
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
                eprintln!("failed to open file {path:?}: {e:?}");
                continue;
            }
        };

        let maybe_datetime = match path.extension().and_then(OsStr::to_str).map(str::to_ascii_lowercase).as_deref() {
            Some("jpg") | Some("tif") | Some("tiff") => match exif_datetime(path) {
                Ok(dt) => Some(dt),
                Err(e) => {
                    eprintln!("{path:?}: Couldn't get EXIF DateTime: {e:?}");
                    None
                }
            },
            _ => None,
        };

        let datetime = maybe_datetime.unwrap_or_else(|| mtime_datetime(&file));

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

        if args.dry_run {
            println!("{path:?} -> {new_path:?}");
        } else if let Err(e) = std::fs::copy(path, &new_path) {
            eprintln!("failed to copy {path:?} to {new_path:?}: {e}");
            continue;
        }
    }

    Ok(())
}
