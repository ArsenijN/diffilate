# diffilate
![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

Diff your files to save some space!

## About diffilate

Diffilate saves the differences between two files into a `.bdiff` file. You can then keep only the 1st file and the `.bdiff` to reconstruct the 2nd file at any time.

Diffilate is effective when you have two copies of a file damaged "in different ways" (e.g. from a failing flash drive with unstable reads), and want to keep both copies without storing them in full.

Diffilate handles files of different sizes in both directions — file1 longer, file2 longer, or equal. All cases produce a correct reconstructed output.

## Usage

```
diffilate file1 file2              # Diff and write file1.bdiff
diffilate --redo file1 diff.bdiff  # Reconstruct file2 from file1 + diff
```

## About .bdiff

BetterDIFFerence — the `.bdiff` format used by diffilate.

- Header: `DIFF` magic + version byte + max file size (u64 le) + flags byte
- Diff records: absolute offset (u64 le) + run length (u8) + changed bytes from file2
- RLE-grouped: consecutive differing bytes are batched into runs of up to 254 bytes
- Maximum addressable file size: 18,446,744,073,709,551,615 bytes (16 EiB)
- Format version is stored in the header for forward/backward compatibility

## Why?

I became the owner of corrupted data from a bad flash drive. The store wouldn't accept a return ("it works!" — yes, at 0.5 MB/s, thanks Foxtrot), and because I'd copied the data twice off the dying drive, both copies were damaged in different ways due to unstable reads from degrading memory chips. Rather than try to figure out which copy had which correct bytes, I wrote diffilate to store a compact diff between them — keeping both versions while only paying the storage cost of the differences.

---

### Improvements for the future:
- [ ] Better `.bdiff` compression (beyond RLE grouping)
- [ ] Reduce address field size for small files (4-byte offsets when file < 4 GiB, etc.)
- [ ] Streaming diff mode (avoid loading all chunks into RAM before diffing)
- [ ] Verify mode: check that file1 + diff produces a file matching a stored checksum

### Known problems:
- [ ] No backward compatibility with V1 DIFF (headerless format)

---

Diffilate like distillate :>
