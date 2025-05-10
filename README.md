# diffilate
![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

Diff your files to save some space! 

## About diffiate

BetterDIFFerence, aka .bdiff - file extension that used for diffilate
To view advanced info about files - consider to view `about bdiff.txt`

## Why?

I recently became the owner of corrupted data due to a bad flash drive. They don't want to return the money (a case of "It works!" but its speed is now ~0.5 MB/s, and the store expects the status "not working at all"... Foxtrot has a "very" good user experience in terms of returns), and because of the double copy from this flash drive, the data was damaged "in different ways" (there was probably an unstable reading due to degradation of the memory chips), and because of my laziness to check where the correct data is (the flash drive was bit rotted), I decided to make a bdiff to leave copies of both files, while being able to save space

Diffilate can handle different file sizes, but only when the first file is larger than the second, otherwise... (see for yourself)

Current improvements for the future:
- [ ] Make .bdiff compression better
- [ ] Check if .bdiff has a DIFF version to maintain compatibility with old .bdiff before switching to a new compression method
