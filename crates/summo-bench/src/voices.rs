//! Where speaker embeddings should live.
//!
//! Transcripts are files because a user has to be able to grep and edit them (ADR 0002). Embeddings
//! are the opposite kind of data: nobody reads a 192-dimensional vector, and the only operation
//! that matters is "compare all of them against a handful of centroids" — which is what a
//! correction sweep does across the entire history.
//!
//! So the question is real and worth measuring rather than assuming. This compares the three
//! honest options on the operation that actually hurts:
//!
//! * **JSON per meeting** — what the code does today.
//! * **Binary per meeting** — the same layout with the encoding fixed: a fixed-size header and raw
//!   `f32` records, so loading is a read rather than a parse.
//! * **SQLite** — one table, one row per utterance, vector as a blob.
//!
//! The workload is one full resweep: load every vector and score it against every centroid.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::Result;

/// CAM++ and ERes2NetV2 both emit 192 dimensions.
const DIMS: usize = 192;

/// People in the voice book, each with several centroids — the comparisons one vector faces.
const CENTROIDS: usize = 80;

pub struct Options {
    pub meetings: usize,
    pub utterances_per_meeting: usize,
}

#[derive(Debug)]
struct Measurement {
    name: &'static str,
    bytes: u64,
    write_ms: f64,
    sweep_ms: f64,
}

pub fn run(options: &Options) -> Result<()> {
    let total = options.meetings * options.utterances_per_meeting;
    println!(
        "{} meetings x {} utterances = {total} vectors of {DIMS} dims\n",
        options.meetings, options.utterances_per_meeting
    );

    let data = generate(options);
    let centroids: Vec<Vec<f32>> = (0..CENTROIDS).map(|i| vector(i as u64 * 7919)).collect();

    let dir = tempfile::tempdir()?;
    let results = [
        json(dir.path(), &data, &centroids)?,
        binary(dir.path(), &data, &centroids)?,
        sqlite(dir.path(), &data, &centroids)?,
    ];

    println!(
        "{:<10} {:>12} {:>12} {:>12}",
        "format", "on disk", "write", "full sweep"
    );
    for r in &results {
        println!(
            "{:<10} {:>12} {:>10.0} ms {:>9.0} ms",
            r.name,
            human(r.bytes),
            r.write_ms,
            r.sweep_ms
        );
    }

    let best = results
        .iter()
        .min_by(|a, b| a.sweep_ms.total_cmp(&b.sweep_ms))
        .expect("three formats were measured");
    println!("\nfastest sweep: {}", best.name);
    Ok(())
}

/// Vectors grouped the way meetings group them.
fn generate(options: &Options) -> Vec<Vec<Vec<f32>>> {
    (0..options.meetings)
        .map(|m| {
            (0..options.utterances_per_meeting)
                .map(|u| vector((m * options.utterances_per_meeting + u) as u64))
                .collect()
        })
        .collect()
}

/// A deterministic pseudo-random unit vector, so every format sees identical data.
fn vector(seed: u64) -> Vec<f32> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut out = Vec::with_capacity(DIMS);
    for _ in 0..DIMS {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push((state >> 33) as f32 / f32::from(u16::MAX) - 1.0);
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    out.iter().map(|x| x / norm).collect()
}

fn json(dir: &Path, data: &[Vec<Vec<f32>>], centroids: &[Vec<f32>]) -> Result<Measurement> {
    let root = dir.join("json");
    std::fs::create_dir_all(&root)?;

    let start = Instant::now();
    for (i, meeting) in data.iter().enumerate() {
        let path = root.join(format!("{i}.json"));
        std::fs::write(&path, serde_json::to_vec(meeting)?)?;
    }
    let write_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let mut checksum = 0.0f32;
    for i in 0..data.len() {
        let text = std::fs::read_to_string(root.join(format!("{i}.json")))?;
        let meeting: Vec<Vec<f32>> = serde_json::from_str(&text)?;
        checksum += score(&meeting, centroids);
    }
    let sweep_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::hint::black_box(checksum);

    Ok(Measurement {
        name: "json",
        bytes: dir_size(&root),
        write_ms,
        sweep_ms,
    })
}

fn binary(dir: &Path, data: &[Vec<Vec<f32>>], centroids: &[Vec<f32>]) -> Result<Measurement> {
    let root = dir.join("bin");
    std::fs::create_dir_all(&root)?;

    let start = Instant::now();
    for (i, meeting) in data.iter().enumerate() {
        let mut file = std::io::BufWriter::new(std::fs::File::create(root.join(format!("{i}.vec")))?);
        file.write_all(&(DIMS as u32).to_le_bytes())?;
        file.write_all(&(meeting.len() as u32).to_le_bytes())?;
        for vector in meeting {
            for value in vector {
                file.write_all(&value.to_le_bytes())?;
            }
        }
        file.flush()?;
    }
    let write_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let mut checksum = 0.0f32;
    for i in 0..data.len() {
        let mut file = std::fs::File::open(root.join(format!("{i}.vec")))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let dims = u32::from_le_bytes(header[..4].try_into()?) as usize;
        let count = u32::from_le_bytes(header[4..].try_into()?) as usize;

        let mut bytes = vec![0u8; dims * count * 4];
        file.read_exact(&mut bytes)?;
        let meeting: Vec<Vec<f32>> = bytes
            .chunks_exact(dims * 4)
            .map(|row| {
                row.chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().expect("four bytes")))
                    .collect()
            })
            .collect();
        checksum += score(&meeting, centroids);
    }
    let sweep_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::hint::black_box(checksum);

    Ok(Measurement {
        name: "binary",
        bytes: dir_size(&root),
        write_ms,
        sweep_ms,
    })
}

fn sqlite(dir: &Path, data: &[Vec<Vec<f32>>], centroids: &[Vec<f32>]) -> Result<Measurement> {
    let path = dir.join("voices.db");
    let mut db = rusqlite::Connection::open(&path)?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE vectors (meeting INTEGER NOT NULL, seq INTEGER NOT NULL, embedding BLOB NOT NULL);
         CREATE INDEX vectors_meeting ON vectors(meeting);",
    )?;

    let start = Instant::now();
    {
        let tx = db.transaction()?;
        {
            let mut insert =
                tx.prepare("INSERT INTO vectors (meeting, seq, embedding) VALUES (?1, ?2, ?3)")?;
            for (m, meeting) in data.iter().enumerate() {
                for (seq, vector) in meeting.iter().enumerate() {
                    let blob: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                    insert.execute(rusqlite::params![m as i64, seq as i64, blob])?;
                }
            }
        }
        tx.commit()?;
    }
    let write_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let mut checksum = 0.0f32;
    {
        let mut select = db.prepare("SELECT embedding FROM vectors")?;
        let mut rows = select.query([])?;
        while let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(0)?;
            let vector: Vec<f32> = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().expect("four bytes")))
                .collect();
            checksum += best_similarity(&vector, centroids);
        }
    }
    let sweep_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::hint::black_box(checksum);

    Ok(Measurement {
        name: "sqlite",
        bytes: file_len(&path) + file_len(&path.with_extension("db-wal")),
        write_ms,
        sweep_ms,
    })
}

/// The work a resweep actually does: every vector against every centroid.
fn score(meeting: &[Vec<f32>], centroids: &[Vec<f32>]) -> f32 {
    meeting
        .iter()
        .map(|v| best_similarity(v, centroids))
        .sum()
}

fn best_similarity(vector: &[f32], centroids: &[Vec<f32>]) -> f32 {
    centroids
        .iter()
        .map(|c| c.iter().zip(vector).map(|(a, b)| a * b).sum::<f32>())
        .fold(f32::MIN, f32::max)
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn dir_size(root: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(_) => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

fn human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}
