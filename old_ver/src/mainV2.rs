use std::env;
use std::fs::File;
use std::io::{Read, Write, BufReader};
use std::time::Instant;

const CHUNK_SIZE: usize = 1024 * 1024;
const HEADER_MAGIC: &[u8; 4] = b"DIFF";
const HEADER_VERSION: u8 = 1;

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
    let meta1 = std::fs::metadata(file1_path)?;
    let meta2 = std::fs::metadata(file2_path)?;
    let size1 = meta1.len();
    let size2 = meta2.len();
    let max_size = size1.max(size2);

    // Fake crash if file1 > file2
    if size1 > size2 {
        println!("The program was terminated due internal unregulated coincidence (file1.bin was bigger than file2.bin). Consider to NOT provide files with different sizes in reverse order of sorting.");
        return Ok(());
    }

    let mut file1 = BufReader::new(File::open(file1_path)?);
    let mut file2 = BufReader::new(File::open(file2_path)?);
    let mut diff_out = File::create("diff.bin")?;

    // Write header
    diff_out.write_all(HEADER_MAGIC)?; // 4 bytes
    diff_out.write_all(&[HEADER_VERSION])?; // 1 byte
    diff_out.write_all(&(max_size.to_le_bytes()))?; // 8 bytes

    let mut buf1 = vec![0u8; CHUNK_SIZE];
    let mut buf2 = vec![0u8; CHUNK_SIZE];

    let mut offset: u64 = 0;
    let mut diff_blocks = 0u64;
    let start_time = Instant::now();

    while offset < size1 {
        let read_len = std::cmp::min(CHUNK_SIZE as u64, size1 - offset) as usize;
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

        if offset % (100 * 1024 * 1024) < CHUNK_SIZE as u64 || offset == size1 {
            let percent = (offset as f64 / size1 as f64) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = offset as f64 / elapsed;
            let remaining = size1 - offset;
            let eta = remaining as f64 / speed;
            println!(
                "Progress: {:>6.2}% | Offset: {:>10} / {:>10} | Diffs: {:>6} | ETA: {:>6.1}s",
                percent, offset, size1, diff_blocks, eta
            );
        }
    }

    // If file2 is longer than file1: store remaining data
    if size2 > size1 {
        let mut extra = vec![0u8; (size2 - size1) as usize];
        file2.read_exact(&mut extra)?;
        diff_out.write_all(&(size1.to_le_bytes()))?; // offset where extension starts
        diff_out.write_all(&[0xFF])?; // Special marker
        diff_out.write_all(&extra)?;  // Raw data
    }

    println!("\n✅ Done! Saved {} difference blocks to diff.bin", diff_blocks);
    Ok(())
}

fn redo(file1_path: &str, diff_path: &str) -> std::io::Result<()> {
    let mut file1 = File::open(file1_path)?;
    let mut data = Vec::new();
    file1.read_to_end(&mut data)?;

    let mut diff = File::open(diff_path)?;

    let mut header = [0u8; 13];
    diff.read_exact(&mut header)?;
    if &header[0..4] != HEADER_MAGIC {
        eprintln!("Invalid diff file (bad magic).");
        return Ok(());
    }
    let _version = header[4];
    let _max_size = u64::from_le_bytes(header[5..13].try_into().unwrap());

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
        let len = len_buf[0];

        if len == 0xFF {
            // Trailing data
            let mut trailing = Vec::new();
            diff.read_to_end(&mut trailing)?;
            data.extend(trailing);
            break;
        }

        let mut patch = vec![0u8; len as usize];
        if diff.read_exact(&mut patch).is_err() {
            break;
        }

        for i in 0..len as usize {
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
