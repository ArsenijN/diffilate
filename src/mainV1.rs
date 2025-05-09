use std::env;
use std::fs::File;
use std::io::{Read, Write, BufReader};
use std::time::Instant;

const CHUNK_SIZE: usize = 1024 * 1024; // 1MB

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        print_usage(&args[0]);
        return Ok(());
    }

    if args[1] == "--redo" {
        if args.len() != 4 {
            eprintln!("Error: --redo requires 2 arguments");
            print_usage(&args[0]);
            return Ok(());
        }
        return redo(&args[2], &args[3]);
    } else {
        if args.len() != 3 {
            eprintln!("Error: diff mode requires 2 arguments");
            print_usage(&args[0]);
            return Ok(());
        }
        return compare(&args[1], &args[2]);
    }
}

fn print_usage(program: &str) {
    println!("Usage:");
    println!("  {} file1.bin file2.bin         # Compare and save diff.bin", program);
    println!("  {} --redo file1.bin diff.bin   # Apply diff.bin to file1 and save file1_diff.bin", program);
}

fn compare(file1_path: &str, file2_path: &str) -> std::io::Result<()> {
    let file1_meta = std::fs::metadata(file1_path)?;
    let file2_meta = std::fs::metadata(file2_path)?;
    let total_len = std::cmp::min(file1_meta.len(), file2_meta.len());

    let mut file1 = BufReader::new(File::open(file1_path)?);
    let mut file2 = BufReader::new(File::open(file2_path)?);
    let mut diff_out = File::create("diff.bin")?;

    let mut buf1 = vec![0u8; CHUNK_SIZE];
    let mut buf2 = vec![0u8; CHUNK_SIZE];

    let mut offset: u64 = 0;
    let mut diff_blocks = 0u64;
    let start_time = Instant::now();

    while offset < total_len {
        let read_len = std::cmp::min(CHUNK_SIZE as u64, total_len - offset) as usize;
        file1.read_exact(&mut buf1[..read_len])?;
        file2.read_exact(&mut buf2[..read_len])?;

        let mut i = 0;
        while i < read_len {
            if buf1[i] != buf2[i] {
                let start = i;
                let mut run_len = 0;
                while i < read_len && buf1[i] != buf2[i] && run_len < 255 {
                    run_len += 1;
                    i += 1;
                }

                let abs_offset = offset + start as u64;
                diff_out.write_all(&abs_offset.to_le_bytes())?;
                diff_out.write_all(&[run_len as u8])?;
                diff_out.write_all(&buf2[start..start + run_len])?;

                diff_blocks += 1;
            } else {
                i += 1;
            }
        }

        offset += read_len as u64;

        if offset % (100 * 1024 * 1024) < CHUNK_SIZE as u64 || offset == total_len {
            let percent = (offset as f64 / total_len as f64) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = offset as f64 / elapsed;
            let remaining = total_len - offset;
            let eta = remaining as f64 / speed;
            println!(
                "Progress: {:>6.2}% | Offset: {:>10} / {:>10} | Diffs: {:>6} | ETA: {:>6.1}s",
                percent,
                offset,
                total_len,
                diff_blocks,
                eta
            );
        }
    }

    println!("\n✅ Done! Saved {} difference blocks to diff.bin", diff_blocks);
    Ok(())
}

fn redo(file1_path: &str, diff_path: &str) -> std::io::Result<()> {
    let mut file1 = File::open(file1_path)?;
    let mut data = Vec::new();
    file1.read_to_end(&mut data)?;

    let mut diff = File::open(diff_path)?;
    let mut applied = 0u64;

    loop {
        let mut offset_buf = [0u8; 8];
        if diff.read_exact(&mut offset_buf).is_err() {
            break;
        }
        let offset = u64::from_le_bytes(offset_buf);

        let mut len_buf = [0u8; 1];
        if diff.read_exact(&mut len_buf).is_err() {
            break;
        }
        let len = len_buf[0] as usize;

        let mut patch = vec![0u8; len];
        if diff.read_exact(&mut patch).is_err() {
            break;
        }

        for i in 0..len {
            if (offset as usize + i) < data.len() {
                data[offset as usize + i] = patch[i];
            }
        }

        applied += 1;
    }

    let mut out = File::create("file1_diff.bin")?;
    out.write_all(&data)?;

    println!("✅ Patching complete. Saved to file1_diff.bin ({} blocks applied)", applied);
    Ok(())
}
