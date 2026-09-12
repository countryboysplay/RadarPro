# Fixtures

This directory will hold real NEXRAD Level II (Archive II) test fixtures
used by `nexrad-level2` and `radar-cli` integration tests.

## Status: S00 placeholder

No fixture data exists yet. Stage S00 ("foundation") only establishes the
directory; it does not decode or validate any radar files.

Real fixtures — small, real NEXRAD Level II excerpts with documented
provenance (site, volume scan time, source), used to test Archive II /
Message 31 decoding, missing-value handling, and range folding — will be
added in stage S01. Per `GLOBAL_CONTRACT.md`, no NEXRAD parsing behavior
should be invented ahead of that fixture-backed verification.
