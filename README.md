# diffilate
![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

Diff your files to save some space! 

## About diffiate

Diffilate saves differences between files in .bdiff file. When completed, you can keep only 1st specified file and .bdiff to make 2nd file

Diffilate effective when you have 2 copies of files that have same size (or different - see below) from failing media, and you need (for some reason) to keep both copies (maybe for later repair attempt)

Diffilate can handle different file sizes, but only when the first file is larger than the second, otherwise... (see for yourself) **(do not use that feature for now unless you sure that output 2nd file from 1st and diff is equal to original 2nd file)**

## About .bdiff file extension

BetterDIFFerence, aka .bdiff - file extension that used for diffilate
To view advanced info about files - consider to view `about bdiff.txt`

## Why?

I recently became the owner of corrupted data due to a bad flash drive. They don't want to return the money (a case of "It works!" but its speed is now ~0.5 MB/s, and the store expects the status "not working at all"... Foxtrot has a "very" good user experience in terms of returns), and because of the double copy from this flash drive, the data was damaged "in different ways" (there was probably an unstable reading due to degradation of the memory chips), and because of my laziness to check where the correct data is (the flash drive was bit rotted), I decided to make a bdiff to leave copies of both files, while being able to save space

---
### Current improvements for the future:
- [ ] Make .bdiff compression better
- [ ] Check if .bdiff has a DIFF version inside to maintain compatibility with old .bdiff before switching to a new compression method
- [ ] Make attempt about reducing of address size for smaller files by header or constant for file size

### Known problems:
- [ ] Slowness on big amount of diffs, needs fixing
- [ ] No backwards compatibility with V1 DIFF :<
- [ ] Broken different size files (wrong output file from diff if input file sizes is not the same: len(file1.bin)≠len(file2.bin))
- [ ] "The program was terminated due internal unregulated coincidence (file1.bin was bigger than file2.bin). Consider to NOT provide files with different sizes in reverse order of sorting." message was overkill, I know
---

Diffilate like distillate :>
