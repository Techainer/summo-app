//! Does a vector index beat a linear scan for "whose voice is this?"
//!
//! `voices.rs` measured *storage*: where the embedding history should live. This measures *search*,
//! which is a different claim and the one actually in dispute. The intuition against a scan is
//! sound in general — indexes are how search gets fast — so the question deserves an experiment
//! rather than an argument.
//!
//! Turso/libSQL is the right thing to test it with. It has native vector types (`F32_BLOB(n)`),
//! `vector_distance_cos()`, and a DiskANN index built by `libsql_vector_idx()`, all embeddable with
//! no server. If an ANN index wins anywhere in Summo's range, it wins here.
//!
//! Three things get measured at each size, because speed alone would be a misleading answer:
//!
//! * **Latency** — the scan, the same query in libSQL without an index, and with one.
//! * **Recall@1** — how often the index returns the *same* person the exact scan does. An index
//!   that is fast and wrong is not a faster index, it is a different feature. Misidentifying a
//!   speaker puts the wrong name on a sentence somebody said, so this is the number that decides.
//! * **Build cost** — time and bytes to construct the index, paid on every correction that moves a
//!   centroid.
//!
//! Run with `--features turso`.

use std::time::Instant;

use anyhow::{Context, Result};

/// CAM++ and ERes2NetV2 both emit 192 dimensions.
const DIMS: usize = 192;

/// Centroids a person accumulates — one per way their voice reaches the microphone.
const CENTROIDS_PER_PERSON: usize = 8;

/// Probes per measurement. Each is a separate "who just spoke?" query.
const PROBES: usize = 200;

pub struct Options {
    /// Voice-book sizes to try, in people.
    pub people: Vec<usize>,
}

struct Row {
    people: usize,
    centroids: usize,
    scan_us: f64,
    libsql_scan_us: f64,
    indexed_us: f64,
    recall: f64,
    build_ms: f64,
    bytes: u64,
}

pub fn run(options: &Options) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    runtime.block_on(measure(options))
}

async fn measure(options: &Options) -> Result<()> {
    println!(
        "identification: one probe against every centroid, {DIMS} dims, {PROBES} probes per cell\n"
    );

    let mut rows = Vec::new();
    for &people in &options.people {
        rows.push(one_size(people).await?);
    }

    println!(
        "{:>7} {:>10} {:>11} {:>13} {:>11} {:>9} {:>10} {:>9}",
        "people", "centroids", "scan", "libsql scan", "indexed", "recall@1", "build", "on disk"
    );
    for r in &rows {
        println!(
            "{:>7} {:>10} {:>8.0} µs {:>10.0} µs {:>8.0} µs {:>8.1}% {:>7.0} ms {:>9}",
            r.people,
            r.centroids,
            r.scan_us,
            r.libsql_scan_us,
            r.indexed_us,
            r.recall * 100.0,
            r.build_ms,
            human(r.bytes),
        );
    }

    println!("\nRecall is measured against the exact scan: the fraction of probes where the");
    println!("index returned the same centroid the scan did. Anything below 100% is a voice");
    println!("attributed to the wrong person.");
    Ok(())
}

async fn one_size(people: usize) -> Result<Row> {
    let count = people * CENTROIDS_PER_PERSON;
    let centroids: Vec<Vec<f32>> = (0..count).map(|i| vector(i as u64 * 31)).collect();
    let probes: Vec<Vec<f32>> = (0..PROBES)
        .map(|i| vector(900_000 + i as u64 * 7_919))
        .collect();

    // The reference answer. Everything else is judged against this.
    let start = Instant::now();
    let truth: Vec<usize> = probes.iter().map(|p| nearest(p, &centroids)).collect();
    let scan_us = start.elapsed().as_secs_f64() * 1e6 / PROBES as f64;

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("voices.db");
    let db = libsql::Builder::new_local(&path).build().await?;
    let conn = db.connect()?;

    conn.execute_batch(&format!(
        "CREATE TABLE centroids (id INTEGER PRIMARY KEY, embedding F32_BLOB({DIMS}) NOT NULL);"
    ))
    .await?;

    conn.execute("BEGIN", ()).await?;
    for (id, c) in centroids.iter().enumerate() {
        conn.execute(
            "INSERT INTO centroids (id, embedding) VALUES (?1, vector32(?2))",
            libsql::params![id as i64, literal(c)],
        )
        .await?;
    }
    conn.execute("COMMIT", ()).await?;

    // Unindexed: libSQL doing the same arithmetic the scan does, to separate the cost of the
    // index from the cost of going through a database at all.
    let libsql_scan_us = time_query(
        &conn,
        "SELECT id FROM centroids ORDER BY vector_distance_cos(embedding, vector32(?1)) LIMIT 1",
        &probes,
        None,
    )
    .await?
    .0;

    let start = Instant::now();
    conn.execute(
        "CREATE INDEX centroids_idx ON centroids (libsql_vector_idx(embedding))",
        (),
    )
    .await?;
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;

    let (indexed_us, hits) = time_query(
        &conn,
        "SELECT id FROM vector_top_k('centroids_idx', vector32(?1), 1)",
        &probes,
        Some(&truth),
    )
    .await?;

    drop(conn);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    Ok(Row {
        people,
        centroids: count,
        scan_us,
        libsql_scan_us,
        indexed_us,
        recall: hits as f64 / PROBES as f64,
        build_ms,
        bytes,
    })
}

/// Time a single-result query over every probe, optionally scoring against the exact answer.
async fn time_query(
    conn: &libsql::Connection,
    sql: &str,
    probes: &[Vec<f32>],
    truth: Option<&[usize]>,
) -> Result<(f64, usize)> {
    let stmt = conn.prepare(sql).await?;
    let mut hits = 0usize;
    let start = Instant::now();
    for (i, probe) in probes.iter().enumerate() {
        stmt.reset();
        let mut rows = stmt.query(libsql::params![literal(probe)]).await?;
        let got = match rows.next().await? {
            Some(row) => usize::try_from(row.get::<i64>(0)?).unwrap_or(usize::MAX),
            None => usize::MAX,
        };
        if let Some(truth) = truth
            && truth[i] == got
        {
            hits += 1;
        }
    }
    let per_probe_us = start.elapsed().as_secs_f64() * 1e6 / probes.len() as f64;
    Ok((per_probe_us, hits))
}

/// libSQL parses vectors from a `'[a, b, c]'` text literal.
fn literal(v: &[f32]) -> String {
    let mut out = String::with_capacity(v.len() * 12 + 2);
    out.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{x}"));
    }
    out.push(']');
    out
}

/// The exact answer: index of the closest centroid by cosine similarity.
///
/// Vectors are unit-length, so the dot product *is* the cosine and the largest one wins.
fn nearest(probe: &[f32], centroids: &[Vec<f32>]) -> usize {
    let mut best = 0usize;
    let mut best_score = f32::MIN;
    for (i, c) in centroids.iter().enumerate() {
        let score: f32 = c.iter().zip(probe).map(|(a, b)| a * b).sum();
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    best
}

/// A deterministic pseudo-random unit vector, matching `voices.rs` so sizes are comparable.
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

fn human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_vectors_are_unit_length() {
        let v = vector(42);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
    }

    #[test]
    fn nearest_finds_an_exact_match() {
        let centroids: Vec<Vec<f32>> = (0..10).map(|i| vector(i * 31)).collect();
        // A probe identical to one centroid must return that centroid.
        assert_eq!(nearest(&centroids[7].clone(), &centroids), 7);
    }

    #[test]
    fn literal_round_trips_through_text() {
        assert_eq!(literal(&[1.0, -0.5]), "[1,-0.5]");
    }
}
