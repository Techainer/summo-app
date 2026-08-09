//! Is a search index worth its complexity?
//!
//! Obsidian-style tools search by scanning files, and for most vaults that is genuinely fast enough.
//! A SQLite index is faster but adds a second source of truth that can drift, corrupt, or need
//! migrating. The honest way to choose is to measure the actual corpus rather than assume, so this
//! module generates a vault of realistic size and times both approaches.
//!
//! The numbers that matter are not just "which is faster" but *whether the slower one is fast
//! enough* — 30 ms and 3 ms are both instant to a person, and the simpler design wins ties.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Roughly one hour of meeting speech: ~9,000 words.
const WORDS_PER_MEETING: usize = 9_000;

/// Vocabulary for synthetic transcripts. Vietnamese, because multi-byte text is the case where a
/// naive byte scan and a real search diverge most.
const VOCAB: &[&str] = &[
    "anh",
    "chị",
    "mình",
    "nên",
    "dùng",
    "cái",
    "này",
    "cho",
    "phần",
    "lõi",
    "vấn",
    "đề",
    "là",
    "API",
    "còn",
    "thiếu",
    "tuần",
    "sau",
    "sẽ",
    "xong",
    "khách",
    "hàng",
    "phản",
    "hồi",
    "rất",
    "tốt",
    "nhưng",
    "cần",
    "thêm",
    "thời",
    "gian",
    "để",
    "kiểm",
    "tra",
    "lại",
    "toàn",
    "bộ",
    "hệ",
    "thống",
    "trước",
    "khi",
    "triển",
    "khai",
    "chính",
    "thức",
    "team",
    "backend",
    "deadline",
    "sprint",
    "review",
    "ngân",
    "sách",
    "quý",
    "tới",
    "tăng",
    "trưởng",
    "người",
    "dùng",
    "mới",
];

/// A word that appears in exactly one meeting, so a search for it has a known answer.
const NEEDLE: &str = "kaleidoscope";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultMetrics {
    pub meetings: usize,
    pub total_bytes: u64,
    /// Full-text search by scanning every file, first run with a cold page cache.
    pub scan_cold_ms: f64,
    /// The same search once the OS has the files cached — the realistic steady state.
    pub scan_warm_ms: f64,
    /// Warm search across 8 threads, which is what a real grep-style tool does. Comparing a
    /// single-threaded scan against a database would rig the result in the database's favour.
    pub scan_parallel_ms: f64,
    /// Listing the library: read each file's frontmatter only.
    pub frontmatter_scan_ms: f64,
    /// Building a SQLite FTS5 index from scratch.
    pub index_build_ms: f64,
    /// The same search through the index.
    pub index_query_ms: f64,
    pub index_bytes: u64,
    /// Sanity check: both approaches must find the same thing.
    pub hits_scan: usize,
    pub hits_index: usize,
}

impl VaultMetrics {
    #[must_use]
    pub fn to_markdown(runs: &[Self]) -> String {
        let mut out = String::new();
        out.push_str("| Meetings | Corpus | Scan 1 thread | Scan 8 threads | List | Index build | Index query | Index size |\n");
        out.push_str("|---:|---:|---:|---:|---:|---:|---:|---:|\n");
        for m in runs {
            out.push_str(&format!(
                "| {} | {} | {:.0} ms | {:.0} ms | {:.0} ms | {:.0} ms | {:.1} ms | {} |\n",
                m.meetings,
                human(m.total_bytes),
                m.scan_warm_ms,
                m.scan_parallel_ms,
                m.frontmatter_scan_ms,
                m.index_build_ms,
                m.index_query_ms,
                human(m.index_bytes),
            ));
        }
        out
    }
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.0} {}", UNITS[u])
}

/// Deterministic pseudo-random word picker, so runs are comparable.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0 >> 33
    }
}

/// Write `count` synthetic meeting files into `dir`.
pub fn generate(dir: &Path, count: usize) -> Result<u64> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut rng = Lcg(0x5EED);
    let mut total = 0;

    for i in 0..count {
        let mut body = String::with_capacity(WORDS_PER_MEETING * 6);
        body.push_str(&format!(
            "---\nid: meeting-{i:05}\ndate: 2026-{:02}-{:02}T10:00:00+07:00\nduration: 3600\nschema: 1\n---\n# Họp số {i}\n\n## Transcript\n",
            (i % 12) + 1,
            (i % 28) + 1
        ));

        let mut words = 0;
        while words < WORDS_PER_MEETING {
            body.push_str(&format!(
                "**[{:02}:{:02}:00] S1** — ",
                words / 900,
                (words / 15) % 60
            ));
            for _ in 0..15 {
                body.push_str(VOCAB[(rng.next() as usize) % VOCAB.len()]);
                body.push(' ');
                words += 1;
            }
            // Plant the needle in exactly one meeting, halfway through the corpus.
            if i == count / 2 && words > WORDS_PER_MEETING / 2 && !body.contains(NEEDLE) {
                body.push_str(NEEDLE);
                body.push(' ');
            }
            body.push('\n');
        }

        let path = dir.join(format!(
            "2026-{:02}-{:02}-hop-{i:05}.md",
            (i % 12) + 1,
            (i % 28) + 1
        ));
        std::fs::write(&path, &body).with_context(|| format!("cannot write {}", path.display()))?;
        total += body.len() as u64;
    }
    Ok(total)
}

/// Search every file for `needle`, the way a grep-based tool would.
fn scan_search(dir: &Path, needle: &str) -> Result<(usize, Duration)> {
    let start = Instant::now();
    let mut hits = 0;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let body = std::fs::read_to_string(entry.path())?;
        if body.contains(needle) {
            hits += 1;
        }
    }
    Ok((hits, start.elapsed()))
}

/// The same search spread over 8 threads, which is how a real grep-style tool works.
///
/// Eight rather than every core on this machine: the comparison should reflect a laptop, not a
/// 64-core server.
fn scan_search_parallel(dir: &Path, needle: &str) -> Result<(usize, Duration)> {
    use rayon::prelude::*;

    let paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .context("cannot build thread pool")?;

    let start = Instant::now();
    let hits = pool.install(|| {
        paths
            .par_iter()
            .filter(|p| {
                std::fs::read_to_string(p)
                    .map(|b| b.contains(needle))
                    .unwrap_or(false)
            })
            .count()
    });
    Ok((hits, start.elapsed()))
}

/// Read only each file's frontmatter, the way a library listing would.
///
/// This is the operation that decides whether the app can show a meeting list instantly at startup
/// without a database.
fn frontmatter_scan(dir: &Path) -> Result<(usize, Duration)> {
    let start = Instant::now();
    let mut count = 0;
    for entry in std::fs::read_dir(dir)?.flatten() {
        // Frontmatter is at the head of the file; reading a few hundred bytes is enough.
        let body = read_head(&entry.path(), 512)?;
        if body.starts_with("---") {
            count += 1;
        }
    }
    Ok((count, start.elapsed()))
}

fn read_head(path: &Path, bytes: usize) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0; bytes];
    let n = file.read(&mut buf)?;
    buf.truncate(n);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Build a SQLite FTS5 index over the vault and query it.
fn index_and_query(
    dir: &Path,
    db: &Path,
    needle: &str,
) -> Result<(usize, Duration, Duration, u64)> {
    let build_start = Instant::now();
    let conn = rusqlite::Connection::open(db)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE VIRTUAL TABLE segments USING fts5(path UNINDEXED, body);",
    )?;

    {
        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare("INSERT INTO segments (path, body) VALUES (?1, ?2)")?;
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let body = std::fs::read_to_string(&path)?;
            stmt.execute(rusqlite::params![path.display().to_string(), body])?;
        }
        drop(stmt);
        tx.commit()?;
    }
    let build = build_start.elapsed();

    let query_start = Instant::now();
    let hits: usize = conn.query_row(
        "SELECT count(*) FROM segments WHERE segments MATCH ?1",
        [needle],
        |row| row.get(0),
    )?;
    let query = query_start.elapsed();

    drop(conn);
    let size = std::fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    Ok((hits, build, query, size))
}

/// Measure both approaches over a freshly generated vault of `count` meetings.
pub fn evaluate(workdir: &Path, count: usize) -> Result<VaultMetrics> {
    let vault: PathBuf = workdir.join(format!("vault-{count}"));
    let total_bytes = generate(&vault, count)?;

    // First search stands in for a cold cache. It is not a true cold-cache measurement — dropping
    // the page cache needs privileges we should not require — so treat it as an upper bound that
    // includes the cost of reading files the generator just wrote.
    let (hits_scan, cold) = scan_search(&vault, NEEDLE)?;
    let (_, warm) = scan_search(&vault, NEEDLE)?;
    let (hits_parallel, parallel) = scan_search_parallel(&vault, NEEDLE)?;
    debug_assert_eq!(hits_parallel, hits_scan);
    let (_, list) = frontmatter_scan(&vault)?;

    let db = workdir.join(format!("index-{count}.db"));
    std::fs::remove_file(&db).ok();
    let (hits_index, build, query, index_bytes) = index_and_query(&vault, &db, NEEDLE)?;

    Ok(VaultMetrics {
        meetings: count,
        total_bytes,
        scan_cold_ms: cold.as_secs_f64() * 1000.0,
        scan_warm_ms: warm.as_secs_f64() * 1000.0,
        scan_parallel_ms: parallel.as_secs_f64() * 1000.0,
        frontmatter_scan_ms: list.as_secs_f64() * 1000.0,
        index_build_ms: build.as_secs_f64() * 1000.0,
        index_query_ms: query.as_secs_f64() * 1000.0,
        index_bytes,
        hits_scan,
        hits_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_meetings_look_like_meetings() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = generate(tmp.path(), 3).unwrap();

        assert!(
            bytes > 3 * 40_000,
            "an hour of speech should be tens of KB, got {bytes}"
        );
        let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 3);

        let body = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(body.starts_with("---"), "frontmatter must lead the file");
        assert!(body.contains("## Transcript"));
        assert!(
            body.contains("**["),
            "transcript lines should carry timestamps"
        );
    }

    #[test]
    fn the_needle_lands_in_exactly_one_meeting() {
        let tmp = tempfile::tempdir().unwrap();
        generate(tmp.path(), 5).unwrap();
        let (hits, _) = scan_search(tmp.path(), NEEDLE).unwrap();
        assert_eq!(hits, 1, "the search benchmark needs a known answer");
    }

    #[test]
    fn both_approaches_find_the_same_thing() {
        let tmp = tempfile::tempdir().unwrap();
        let m = evaluate(tmp.path(), 5).unwrap();
        assert_eq!(m.hits_scan, 1);
        assert_eq!(
            m.hits_index, m.hits_scan,
            "a benchmark comparing two search methods is worthless if they disagree"
        );
    }

    #[test]
    fn the_parallel_scan_agrees_with_the_serial_one() {
        let tmp = tempfile::tempdir().unwrap();
        generate(tmp.path(), 8).unwrap();
        let (serial, _) = scan_search(tmp.path(), NEEDLE).unwrap();
        let (parallel, _) = scan_search_parallel(tmp.path(), NEEDLE).unwrap();
        assert_eq!(serial, parallel);
    }

    #[test]
    fn frontmatter_scan_reads_only_the_head_of_each_file() {
        let tmp = tempfile::tempdir().unwrap();
        generate(tmp.path(), 5).unwrap();
        let (count, elapsed) = frontmatter_scan(tmp.path()).unwrap();
        let (_, full) = scan_search(tmp.path(), NEEDLE).unwrap();

        assert_eq!(count, 5);
        assert!(
            elapsed <= full,
            "reading 512 bytes per file should not cost more than reading all of it"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(
            generate(a.path(), 2).unwrap(),
            generate(b.path(), 2).unwrap()
        );
    }
}
