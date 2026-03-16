use std::env;
use std::fs::File;
use std::io::{Read, Write, BufReader, Seek, SeekFrom, Error, ErrorKind};
use std::path::Path;
use std::time::Instant;

const CHUNK_SIZE: usize = 1024 * 1024;
const HEADER_MAGIC: &[u8; 4] = b"DIFF";
const HEADER_VERSION: u8 = 6; // Increasing version number

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
    println!("  {} file1.bin file2.bin           # Compare and save file1.bin.bdiff", program);
    println!("  {} --redo file1.bin diff.bdiff   # Apply diff.bdiff to file1 and save file1_diff.bin", program);
}

fn get_output_filename(file_path: &str) -> String {
    // Extract the full filename from the path
    let path = Path::new(file_path);
    let filename = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    
    format!("{}.bdiff", filename)
}

fn compare(file1_path: &str, file2_path: &str) -> std::io::Result<()> {
    let meta1 = std::fs::metadata(file1_path)?;
    let meta2 = std::fs::metadata(file2_path)?;
    let size1 = meta1.len();
    let size2 = meta2.len();
    let max_size = size1.max(size2);

    // Generate output filename based on the first input file
    let output_filename = get_output_filename(file1_path);
    let mut diff_out = File::create(&output_filename)?;

    // Write header
    diff_out.write_all(HEADER_MAGIC)?; // 4 bytes
    diff_out.write_all(&[HEADER_VERSION])?; // 1 byte
    diff_out.write_all(&(max_size.to_le_bytes()))?; // 8 bytes

    // Now handle both cases: file1 ≤ file2 and file1 > file2
    // In V6, we'll always store differences relative to the first file
    // without swapping the order of files based on size
    diff_out.write_all(&[if size1 <= size2 { 0x00 } else { 0x01 }])?; // Flag for which file is larger
    
    let result = process_files(
        file1_path,
        file2_path,
        size1,
        size2,
        &mut diff_out,
        false // We don't use the reversed flag anymore in V6
    )?;

    println!("\n✅ Done! Saved difference blocks to {}", output_filename);
    Ok(())
}

fn process_files(
    file1_path: &str,
    file2_path: &str,
    file1_size: u64,
    file2_size: u64,
    diff_out: &mut File,
    _reversed: bool, // Keep for backward compatibility but not used in V6
) -> std::io::Result<(u64, u64)> {
    let mut file1 = BufReader::new(File::open(file1_path)?);
    let mut file2 = BufReader::new(File::open(file2_path)?);
    
    let mut buf1 = vec![0u8; CHUNK_SIZE];
    let mut buf2 = vec![0u8; CHUNK_SIZE];

    let mut offset: u64 = 0;
    let mut diff_blocks = 0u64;
    let start_time = Instant::now();
    
    // Find the minimum size to compare byte-by-byte
    let min_size = std::cmp::min(file1_size, file2_size);

    // Process the common part of both files
    while offset < min_size {
        let read_len = std::cmp::min(CHUNK_SIZE as u64, min_size - offset) as usize;
        file1.read_exact(&mut buf1[..read_len])?;
        file2.read_exact(&mut buf2[..read_len])?;

        let mut i = 0;
        while i < read_len {
            let mut bytes_equal = buf1[i] == buf2[i];

            if !bytes_equal {
                let start = i;
                let mut run_len = 0;
                
                // Find sequence of different bytes (limited to 255 bytes)
                while i < read_len && !bytes_equal && run_len < 255 {
                    run_len += 1;
                    i += 1;
                    
                    if i < read_len {
                        bytes_equal = buf1[i] == buf2[i];
                    }
                }

                let abs_offset = offset + start as u64;
                diff_out.write_all(&abs_offset.to_le_bytes())?;
                diff_out.write_all(&[run_len as u8])?;
                
                // Always write bytes from file2 (target)
                diff_out.write_all(&buf2[start..start + run_len])?;
                
                diff_blocks += 1;
            } else {
                i += 1;
            }
        }

        offset += read_len as u64;

        if offset % (100 * 1024 * 1024) < CHUNK_SIZE as u64 || offset == min_size {
            let percent = (offset as f64 / min_size as f64) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { offset as f64 / elapsed } else { 0.0 };
            let remaining = min_size - offset;
            let eta = if speed > 0.0 { remaining as f64 / speed } else { 0.0 };
            println!(
                "Progress: {:>6.2}% | Offset: {:>10} / {:>10} | Diffs: {:>6} | ETA: {:>6.1}s",
                percent, offset, min_size, diff_blocks, eta
            );
        }
    }
    
    // Handle the case where one file is longer than the other
    if file1_size != file2_size {
        // Store the marker and size difference
        diff_out.write_all(&(min_size.to_le_bytes()))?; // offset where extension starts
        diff_out.write_all(&[0xFF])?; // Special marker
        
        if file2_size > file1_size {
            // File2 is longer, store the remainder
            let remainder_size = file2_size - file1_size;
            let mut extra = vec![0u8; std::cmp::min(remainder_size, 10_000_000) as usize];
            
            let mut remaining = remainder_size;
            while remaining > 0 {
                let read_size = std::cmp::min(extra.len() as u64, remaining) as usize;
                file2.read_exact(&mut extra[..read_size])?;
                diff_out.write_all(&extra[..read_size])?;
                remaining -= read_size as u64;
            }
        } else {
            // File1 is longer, we need to mark this to handle the redo operation correctly
            // Write a special size marker to indicate file1 is longer and should be truncated
            diff_out.write_all(&file2_size.to_le_bytes())?;
        }
    }
    
    Ok((file1_size, file2_size))

    Ok((smaller_size, larger_size))
}

fn redo(file1_path: &str, diff_path: &str) -> std::io::Result<()> {
    let mut file1 = File::open(file1_path)?;
    let mut data = Vec::new();
    file1.read_to_end(&mut data)?;

    // Generate output filename based on the first input file
    let path = Path::new(file1_path);
    let filename = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let output_filename = format!("{}_diff.bin", filename);

    // Try multiple version formats, starting with the newest
    let mut versions_to_try = vec![6, 5, 4, 3, 2, 1];
    let mut success = false;
    
    while !versions_to_try.is_empty() && !success {
        let version = versions_to_try.remove(0);
        match apply_diff(&mut data, diff_path, version, file1_path, &output_filename) {
            Ok(applied) => {
                println!("✅ Patching complete with format version {}. Saved to {} ({} blocks applied)", 
                         version, output_filename, applied);
                success = true;
            },
            Err(e) => {
                if versions_to_try.is_empty() {
                    return Err(e);
                }
                println!("Info: Trying older format version (v{})...", versions_to_try[0]);
            }
        }
    }

    if !success {
        return Err(Error::new(ErrorKind::InvalidData, "Could not apply diff file with any known format"));
    }
    
    Ok(())
}

fn apply_diff(data: &mut Vec<u8>, diff_path: &str, version: u8, file1_path: &str, output_filename: &str) -> std::io::Result<u64> {
    let mut diff = File::open(diff_path)?;
    let mut applied = 0u64;
    let mut is_reversed = false;
    
    match version {
        1 => {
            // V1: No header, just start reading diffs
            // Nothing to do here, start processing immediately
        },
        2..=6 => {
            // V2-V5: Header with magic and version
            let mut header = [0u8; 13];
            diff.read_exact(&mut header)?;
            
            if &header[0..4] != HEADER_MAGIC {
                return Err(Error::new(ErrorKind::InvalidData, "Invalid diff file (bad magic)"));
            }
            
            let _version = header[4];
            let _max_size = u64::from_le_bytes(header[5..13].try_into().unwrap());
            
            // In V5, check for the reversed flag
            // Handle version specific flags
            if version == 5 {
                // V5: Check for the reversed flag (0xFE)
                let mut flag_buf = [0u8; 1];
                match diff.read_exact(&mut flag_buf) {
                    Ok(_) => {
                        if flag_buf[0] == 0xFE {
                            // Files were processed in reverse order
                            is_reversed = true;
                        } else {
                            // Not a reversed flag, seek back
                            diff.seek(SeekFrom::Current(-1))?;
                        }
                    },
                    Err(_) => {
                        // Couldn't read flag, ignore and continue with normal processing
                        diff.seek(SeekFrom::Current(-1))?;
                    }
                }
            } else if version == 6 {
                // V6: Read a flag that tells us which file was larger
                let mut flag_buf = [0u8; 1];
                diff.read_exact(&mut flag_buf)?;
                
                // 0x01 means first file was larger than second
                // For applying a diff, we don't need to change behavior
                // Just read this flag and continue
            }
            }
        },
        _ => return Err(Error::new(ErrorKind::InvalidData, "Unsupported diff version"))
    }
    
    // Process the diff file
    loop {
        let mut offset_buf = [0u8; 8];
        if let Err(e) = diff.read_exact(&mut offset_buf) {
            if e.kind() == ErrorKind::UnexpectedEof {
                // Reached end of file normally
                break;
            } else {
                // Other error
                return Err(e);
            }
        }
        let offset = u64::from_le_bytes(offset_buf);

        let mut len_buf = [0u8; 1];
        if diff.read_exact(&mut len_buf).is_err() {
            // This shouldn't happen - if we read the offset, we should be able to read the length
            return Err(Error::new(ErrorKind::InvalidData, "Truncated diff file"));
        }
        let len = len_buf[0];

        if len == 0xFF {
            // Special trailing data marker
            if version == 6 {
                // In V6, we have two cases: appending or truncating
                let mut target_size_buf = [0u8; 8];
                
                // First try to read the target size (exists only when truncating)
                if let Ok(_) = diff.read_exact(&mut target_size_buf) {
                    // If we can read 8 more bytes, it's a truncation command (file1 > file2)
                    let target_size = u64::from_le_bytes(target_size_buf);
                    
                    // Truncate the data to the target size
                    if data.len() > target_size as usize {
                        data.truncate(target_size as usize);
                    }
                } else {
                    // No target size, just read the remaining data and append it
                    let mut trailing = Vec::new();
                    diff.read_to_end(&mut trailing)?;
                    data.extend(trailing);
                }
            } else {
                // Pre-V6 behavior
                let mut trailing = Vec::new();
                diff.read_to_end(&mut trailing)?;
                data.extend(trailing);
            }
            break;
        }

        let mut patch = vec![0u8; len as usize];
        if diff.read_exact(&mut patch).is_err() {
            return Err(Error::new(ErrorKind::InvalidData, "Truncated diff file"));
        }

        if is_reversed && version < 6 {
            // Only relevant for V5 and earlier
            // In reversed mode, we're actually replacing bytes in file2 with bytes from file1
            // But since we're applying the diff to file1, we need to keep the bytes from file1
            // So we don't apply the patch in this case
        } else {
            // Normal mode - apply the patch
            // Ensure the data vector is large enough
            if (offset as usize + len as usize) > data.len() {
                data.resize(offset as usize + len as usize, 0);
            }
            
            // Apply the patch
            for i in 0..len as usize {
                data[offset as usize + i] = patch[i];
            }
        }

        applied += 1;
    }

    let mut out = File::create(output_filename)?;
    out.write_all(&data)?;
    
    Ok(applied)
}