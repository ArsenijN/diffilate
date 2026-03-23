use std::env;
use std::fs::File;
use std::io::{Read, Write, BufReader, BufWriter, Seek, SeekFrom, Error, ErrorKind};
use std::path::Path;
use std::time::Instant;
use rayon::prelude::*;

// ── Format constants ──────────────────────────────────────────────────────────
const CHUNK_SIZE: usize = 1024 * 1024;  // 1 MiB per chunk
const BATCH_CHUNKS: usize = 64;         // chunks per parallel batch → 64 MiB RAM per file at once
const HEADER_MAGIC: &[u8; 4] = b"DIFF";
const HEADER_VERSION: u8 = 7;

// V7 header layout (14 bytes total):
//   [0..4]  magic    "DIFF"
//   [4]     version  7
//   [5..13] max_size u64 le  (max of both file sizes)
//   [13]    flags    u8
//             bit 0: file1 was LONGER than file2 (truncation needed on redo)
//             bit 1: RLE grouping enabled (always 1 in V7)
//
// Diff record layout (V2-V7):
//   [0..8]  abs_offset u64 le
//   [8]     run_len    u8   1..=254 = normal patch; 0xFF = size-extension marker
//   [9..9+run_len] replacement bytes from file2
//
// Size-extension record (only present when sizes differ):
//   offset = min(size1, size2)
//   len    = 0xFF
//   if file2 > file1: raw appended bytes follow (until EOF)
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
        "--out-file1" => {
            // Name the output after file1 instead of file2
            if args.len() != 4 {
                eprintln!("Error: --out-file1 requires exactly 2 file arguments");
                print_usage(&args[0]);
                return Ok(());
            }
            let out = bdiff_name(&args[2]);
            compare(&args[2], &args[3], &out)
        }
        _ => {
            if args.len() != 3 {
                eprintln!("Error: diff mode requires exactly 2 arguments");
                print_usage(&args[0]);
                return Ok(());
            }
            // Default: name output after file2
            let out = bdiff_name(&args[2]);
            compare(&args[1], &args[2], &out)
        }
    }
}

fn print_usage(program: &str) {
    println!("diffilate v{}", env!("CARGO_PKG_VERSION"));
    println!("Usage:");
    println!("  {program} file1 file2                 # Diff; output named file2.bdiff");
    println!("  {program} --out-file1 file1 file2     # Same but output named file1.bdiff");
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
        if buf1[i] == buf2[i] { i += 1; continue; }
        let start = i;
        let mut run: Vec<u8> = Vec::with_capacity(254);
        while i < len && buf1[i] != buf2[i] && run.len() < 254 {
            run.push(buf2[i]);
            i += 1;
        }
        records.push(DiffRecord { offset: chunk_base + start as u64, data: run });
    }
    records
}

// ── Compare (write diff) ──────────────────────────────────────────────────────
fn compare(file1_path: &str, file2_path: &str, out_path: &str) -> std::io::Result<()> {
    let size1 = std::fs::metadata(file1_path)?.len();
    let size2 = std::fs::metadata(file2_path)?.len();
    let max_size = size1.max(size2);
    let min_size = size1.min(size2);

    let mut out = BufWriter::new(File::create(out_path)?);

    // Header (14 bytes)
    out.write_all(HEADER_MAGIC)?;
    out.write_all(&[HEADER_VERSION])?;
    out.write_all(&max_size.to_le_bytes())?;
    let flags: u8 = 0b10 | if size1 > size2 { 0b01 } else { 0b00 };
    out.write_all(&[flags])?;

    // ── Batched parallel diff ─────────────────────────────────────────────────
    // We read BATCH_CHUNKS chunks at a time (64 MiB per file = 128 MiB total),
    // diff that batch in parallel, write results, then move to the next batch.
    // Peak RAM = 2 * BATCH_CHUNKS * CHUNK_SIZE regardless of total file size.
    let mut f1 = BufReader::new(File::open(file1_path)?);
    let mut f2 = BufReader::new(File::open(file2_path)?);

    let mut offset: u64 = 0;
    let mut total_records: u64 = 0;
    let mut total_diff_bytes: u64 = 0;
    let start = Instant::now();

    while offset < min_size {
        // Fill one batch
        let mut batch: Vec<(Vec<u8>, Vec<u8>, u64)> = Vec::with_capacity(BATCH_CHUNKS);
        for _ in 0..BATCH_CHUNKS {
            if offset >= min_size { break; }
            let n = (CHUNK_SIZE as u64).min(min_size - offset) as usize;
            let mut b1 = vec![0u8; n];
            let mut b2 = vec![0u8; n];
            f1.read_exact(&mut b1)?;
            f2.read_exact(&mut b2)?;
            batch.push((b1, b2, offset));
            offset += n as u64;
        }

        // Diff batch in parallel
        let results: Vec<Vec<DiffRecord>> = batch
            .par_iter()
            .map(|(b1, b2, base)| encode_chunk(b1, b2, *base))
            .collect();

        // Write results in order
        for records in &results {
            for rec in records {
                out.write_all(&rec.offset.to_le_bytes())?;
                out.write_all(&[rec.data.len() as u8])?;
                out.write_all(&rec.data)?;
                total_records += 1;
                total_diff_bytes += rec.data.len() as u64;
            }
        }

        // Progress
        let elapsed = start.elapsed().as_secs_f64();
        let speed = (offset as f64 / 1024.0 / 1024.0) / elapsed.max(0.001);
        let remaining = min_size - offset;
        let eta = if speed > 0.0 { remaining as f64 / 1024.0 / 1024.0 / speed } else { 0.0 };
        eprint!(
            "\rProgress: {:.1}%  {:.0}/{:.0} MiB  {:.0} MiB/s  ETA {:.1}s  diffs: {}   ",
            offset as f64 / min_size as f64 * 100.0,
            offset as f64 / 1024.0 / 1024.0,
            min_size as f64 / 1024.0 / 1024.0,
            speed,
            eta,
            total_records,
        );
    }
    eprintln!(); // newline after progress

    let elapsed = start.elapsed().as_secs_f64();
    let speed = (min_size as f64 / 1024.0 / 1024.0) / elapsed.max(0.001);
    println!(
        "Diffed {} MiB in {:.2}s  ({:.1} MiB/s) | {} records  ({} diff bytes  ratio {:.4})",
        min_size / 1024 / 1024,
        elapsed,
        speed,
        total_records,
        total_diff_bytes,
        if min_size > 0 { total_diff_bytes as f64 / min_size as f64 } else { 0.0 },
    );

    // Size-extension record
    if size1 != size2 {
        out.write_all(&min_size.to_le_bytes())?;
        out.write_all(&[0xFF_u8])?;

        if size2 > size1 {
            // Stream extra tail of file2 directly to output
            // f2 is already positioned right at min_size from the loop above
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
            out.write_all(&size2.to_le_bytes())?;
            println!("  file1 is longer: truncation to {} bytes recorded", size2);
        }
    }

    out.flush()?;
    println!("✅  Done!  Diff saved to {out_path}");
    Ok(())
}

// ── Redo (apply diff) ─────────────────────────────────────────────────────────
// Streaming: copy file1 to output, then apply patches by seeking in the output
// file directly. Never loads the whole file into RAM.
fn redo(file1_path: &str, diff_path: &str) -> std::io::Result<()> {
    let out_path = {
        let name = Path::new(file1_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        format!("{name}_redo.bin")
    };

    // Step 1: stream-copy file1 → output
    {
        let mut src = BufReader::new(File::open(file1_path)?);
        let mut dst = BufWriter::new(File::create(&out_path)?);
        let mut buf = vec![0u8; 65536];
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 { break; }
            dst.write_all(&buf[..n])?;
        }
    }

    // Step 2: open output for random-write, apply patches by seeking
    let applied = apply_diff_streaming(diff_path, &out_path)?;

    println!("✅  Redo complete → {out_path}  ({applied} patch records applied)");
    Ok(())
}

fn apply_diff_streaming(diff_path: &str, out_path: &str) -> std::io::Result<u64> {
    let mut diff = BufReader::new(File::open(diff_path)?);
    // Open output for read+write (patches may extend it)
    let mut out = File::options().read(true).write(true).open(out_path)?;

    // ── Read header, detect version ───────────────────────────────────────────
    let mut magic = [0u8; 4];
    diff.read_exact(&mut magic)?;

    let version: u8;
    let is_truncation: bool;

    if &magic == HEADER_MAGIC {
        let mut vbuf = [0u8; 1];
        diff.read_exact(&mut vbuf)?;
        version = vbuf[0];

        let mut sbuf = [0u8; 8];
        diff.read_exact(&mut sbuf)?; // consume max_size

        is_truncation = match version {
            1..=4 => false,   // ← was 2..=4
            5 => {
                let mut fb = [0u8; 1];
                match diff.read_exact(&mut fb) {
                    Ok(_) if fb[0] == 0xFE => false,
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
        diff.seek(SeekFrom::Start(0))?;
        version = 1;
        is_truncation = false;
        println!("Format: DIFF V1 (headerless)");
    }

    // ── Apply records ─────────────────────────────────────────────────────────
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
                let mut ts = [0u8; 8];
                diff.read_exact(&mut ts)?;
                let target = u64::from_le_bytes(ts);
                out.set_len(target)?;
                println!("  Truncated to {target} bytes");
            } else {
                // Append tail: seek to end of output, stream from diff
                out.seek(SeekFrom::End(0))?;
                let mut buf = vec![0u8; 65536];
                let mut appended: u64 = 0;
                loop {
                    let n = diff.read(&mut buf)?;
                    if n == 0 { break; }
                    out.write_all(&buf[..n])?;
                    appended += n as u64;
                }
                println!("  Appended {appended} bytes");
            }
            break;
        }

        // Normal patch: seek to offset in output, write replacement bytes
        let mut patch = vec![0u8; len as usize];
        diff.read_exact(&mut patch)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Truncated diff: missing patch bytes"))?;

        // If patch extends beyond current file size, grow it
        let end = offset + len as u64;
        let cur_len = out.seek(SeekFrom::End(0))?;
        if end > cur_len {
            out.set_len(end)?;
        }
        out.seek(SeekFrom::Start(offset))?;
        out.write_all(&patch)?;
        applied += 1;
    }

    Ok(applied)
}
