use std::env;
use std::fs::File;
use std::io::{Read, Write, BufReader, BufWriter, Seek, SeekFrom, Error, ErrorKind};
use std::path::Path;
use std::time::Instant;
use rayon::prelude::*;

// ── Format constants ──────────────────────────────────────────────────────────
const CHUNK_SIZE: usize = 1024 * 1024; // 1 MiB per chunk
const HEADER_MAGIC: &[u8; 4] = b"DIFF";
const HEADER_VERSION: u8 = 7;

// V7 header layout (14 bytes total):
//   [0..4]  magic   "DIFF"
//   [4]     version 7
//   [5..13] max_size  u64 le  (max of both file sizes)
//   [13]    flags   u8
//             bit 0: file1 was LONGER than file2 (truncation needed on redo)
//             bit 1: RLE compression enabled (always 1 in V7)
//
// Diff record layout (V2-V7):
//   [0..8]  abs_offset  u64 le
//   [8]     run_len     u8   1..=254 = normal patch; 0xFF = size-extension
//   [9..9+run_len] bytes from file2
//
// Size-extension record (only when sizes differ):
//   offset  = min(size1, size2)
//   len     = 0xFF
//   if file2 > file1: raw appended bytes follow (read to EOF)
//   if file1 > file2: 8-byte u64 le target_size follows, then EOF

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        print_usage(&args[0]);
        return Ok(());
    }

    match args[1].as_str() {
        "--redo" => {
            if args.len() != 4 {
                eprintln!("Error: --redo requires exactly 2 arguments");
                print_usage(&args[0]);
                return Ok(());
            }
            redo(&args[2], &args[3])
        }
        _ => {
            if args.len() != 3 {
                eprintln!("Error: diff mode requires exactly 2 arguments");
                print_usage(&args[0]);
                return Ok(());
            }
            compare(&args[1], &args[2])
        }
    }
}

fn print_usage(program: &str) {
    println!("diffilate v{}", env!("CARGO_PKG_VERSION"));
    println!("Usage:");
    println!("  {program} file1 file2                 # Diff and write file1.bdiff");
    println!("  {program} --redo file1 diff.bdiff     # Reconstruct file2 from file1 + diff");
}

fn bdiff_name(file_path: &str) -> String {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    format!("{filename}.bdiff")
}

// ── Per-chunk diff record ─────────────────────────────────────────────────────
struct DiffRecord {
    offset: u64,
    data: Vec<u8>,
}

fn encode_chunk(buf1: &[u8], buf2: &[u8], chunk_base: u64) -> Vec<DiffRecord> {
    let len = buf1.len().min(buf2.len());
    let mut records = Vec::new();
    let mut i = 0usize;

    while i < len {
        if buf1[i] == buf2[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut run: Vec<u8> = Vec::with_capacity(254);
        while i < len && buf1[i] != buf2[i] && run.len() < 254 {
            run.push(buf2[i]);
            i += 1;
        }
        records.push(DiffRecord {
            offset: chunk_base + start as u64,
            data: run,
        });
    }
    records
}

// ── Compare (write diff) ──────────────────────────────────────────────────────
fn compare(file1_path: &str, file2_path: &str) -> std::io::Result<()> {
    let size1 = std::fs::metadata(file1_path)?.len();
    let size2 = std::fs::metadata(file2_path)?.len();
    let max_size = size1.max(size2);
    let min_size = size1.min(size2);

    let out_path = bdiff_name(file1_path);
    let mut out = BufWriter::new(File::create(&out_path)?);

    // Header (14 bytes)
    out.write_all(HEADER_MAGIC)?;
    out.write_all(&[HEADER_VERSION])?;
    out.write_all(&max_size.to_le_bytes())?;
    let flags: u8 = 0b10 | if size1 > size2 { 0b01 } else { 0b00 };
    out.write_all(&[flags])?;

    // Read phase: slurp all chunks sequentially (I/O bound, must be in order)
    let total_chunks = ((min_size as usize) + CHUNK_SIZE - 1) / CHUNK_SIZE;
    let mut chunks: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(total_chunks);
    {
        let mut f1 = BufReader::new(File::open(file1_path)?);
        let mut f2 = BufReader::new(File::open(file2_path)?);
        let mut remaining = min_size;
        while remaining > 0 {
            let n = (CHUNK_SIZE as u64).min(remaining) as usize;
            let mut b1 = vec![0u8; n];
            let mut b2 = vec![0u8; n];
            f1.read_exact(&mut b1)?;
            f2.read_exact(&mut b2)?;
            chunks.push((b1, b2));
            remaining -= n as u64;
        }
    }

    let start = Instant::now();

    // Diff phase: parallel over chunks
    let chunk_results: Vec<Vec<DiffRecord>> = chunks
        .par_iter()
        .enumerate()
        .map(|(idx, (b1, b2))| encode_chunk(b1, b2, idx as u64 * CHUNK_SIZE as u64))
        .collect();

    // Write phase: in-order
    let mut total_records: u64 = 0;
    let mut total_diff_bytes: u64 = 0;
    for records in &chunk_results {
        for rec in records {
            out.write_all(&rec.offset.to_le_bytes())?;
            out.write_all(&[rec.data.len() as u8])?;
            out.write_all(&rec.data)?;
            total_records += 1;
            total_diff_bytes += rec.data.len() as u64;
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let speed = (min_size as f64 / 1024.0 / 1024.0) / elapsed.max(0.001);
    println!(
        "Diffed {} MiB in {:.2}s  ({:.1} MiB/s) | {} records  ({} diff bytes  /  {} ratio)",
        min_size / 1024 / 1024,
        elapsed,
        speed,
        total_records,
        total_diff_bytes,
        if min_size > 0 {
            format!("{:.4}", total_diff_bytes as f64 / min_size as f64)
        } else {
            "N/A".into()
        }
    );

    // Size-extension record
    if size1 != size2 {
        out.write_all(&min_size.to_le_bytes())?;
        out.write_all(&[0xFF_u8])?;

        if size2 > size1 {
            // Append extra tail of file2
            let mut f2 = BufReader::new(File::open(file2_path)?);
            f2.seek(SeekFrom::Start(min_size))?;
            let mut buf = vec![0u8; 65536];
            let mut remaining = size2 - size1;
            while remaining > 0 {
                let n = (buf.len() as u64).min(remaining) as usize;
                f2.read_exact(&mut buf[..n])?;
                out.write_all(&buf[..n])?;
                remaining -= n as u64;
            }
            println!("  file2 is longer: +{} appended bytes stored", size2 - size1);
        } else {
            // Truncation: store target size only
            out.write_all(&size2.to_le_bytes())?;
            println!("  file1 is longer: truncation to {} bytes recorded", size2);
        }
    }

    out.flush()?;
    println!("✅  Done!  Diff saved to {out_path}");
    Ok(())
}

// ── Redo (apply diff) ─────────────────────────────────────────────────────────
fn redo(file1_path: &str, diff_path: &str) -> std::io::Result<()> {
    let mut data: Vec<u8> = {
        let mut f = File::open(file1_path)?;
        let mut v = Vec::new();
        f.read_to_end(&mut v)?;
        v
    };

    let out_path = {
        let name = Path::new(file1_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        format!("{name}_redo.bin")
    };

    let applied = apply_diff(&mut data, diff_path)?;

    let mut out = File::create(&out_path)?;
    out.write_all(&data)?;
    println!("✅  Redo complete → {out_path}  ({applied} patch records applied)");
    Ok(())
}

fn apply_diff(data: &mut Vec<u8>, diff_path: &str) -> std::io::Result<u64> {
    let mut diff = BufReader::new(File::open(diff_path)?);

    // Detect version from header
    let mut magic = [0u8; 4];
    diff.read_exact(&mut magic)?;

    let version: u8;
    let is_truncation: bool;

    if &magic == HEADER_MAGIC {
        let mut vbuf = [0u8; 1];
        diff.read_exact(&mut vbuf)?;
        version = vbuf[0];

        let mut sbuf = [0u8; 8];
        diff.read_exact(&mut sbuf)?;
        // _max_size consumed

        is_truncation = match version {
            2..=4 => false,
            5 => {
                // V5: peek for optional 0xFE flag
                let mut fb = [0u8; 1];
                match diff.read_exact(&mut fb) {
                    Ok(_) if fb[0] == 0xFE => false, // reversed flag, treat as normal
                    Ok(_) => { diff.seek(SeekFrom::Current(-1))?; false }
                    Err(_) => false,
                }
            }
            6 => {
                let mut fb = [0u8; 1];
                diff.read_exact(&mut fb)?;
                fb[0] == 0x01
            }
            7 => {
                let mut fb = [0u8; 1];
                diff.read_exact(&mut fb)?;
                (fb[0] & 0x01) != 0
            }
            other => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("Unsupported DIFF version {other}"),
                ));
            }
        };
        println!("Format: DIFF V{version}  (truncation_needed={is_truncation})");
    } else {
        // V1: no header — seek back to start, treat first 4 bytes as offset data
        diff.seek(SeekFrom::Start(0))?;
        version = 1;
        is_truncation = false;
        println!("Format: DIFF V1 (headerless)");
    }

    let mut applied: u64 = 0;

    loop {
        let mut obuf = [0u8; 8];
        match diff.read_exact(&mut obuf) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let offset = u64::from_le_bytes(obuf);

        let mut lbuf = [0u8; 1];
        diff.read_exact(&mut lbuf)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Truncated diff: missing length byte"))?;
        let len = lbuf[0];

        if len == 0xFF {
            if is_truncation {
                // Read 8-byte target size
                let mut ts = [0u8; 8];
                diff.read_exact(&mut ts)?;
                let target = u64::from_le_bytes(ts) as usize;
                if data.len() > target {
                    data.truncate(target);
                    println!("  Truncated to {target} bytes");
                }
            } else {
                // Append tail
                let mut tail = Vec::new();
                diff.read_to_end(&mut tail)?;
                println!("  Appended {} bytes", tail.len());
                data.extend(tail);
            }
            break;
        }

        let mut patch = vec![0u8; len as usize];
        diff.read_exact(&mut patch)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Truncated diff: missing patch bytes"))?;

        let end = offset as usize + len as usize;
        if end > data.len() {
            data.resize(end, 0);
        }
        data[offset as usize..end].copy_from_slice(&patch);
        applied += 1;
    }

    Ok(applied)
}
