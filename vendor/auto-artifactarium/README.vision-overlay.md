# Vision Overlay decoder patch

Based on https://github.com/konkers/auto-artifactarium at commit
`7dafbf0af0bfece23e46382dd3f991d39a5436ac`, version 1.4.0. The upstream MIT
license is included. Cargo selects this copy through the workspace's Git-source
patch; the shared Cargo checkout is unchanged.

Local changes in `src/lib.rs` and `src/crypto.rs`:

- Preserve a position in the known client RNG stream across connection resets.
- Check a bounded window of 64 positions, with eight positions of lookback,
  for every possible server seed before performing the original timestamp search.
  Successful matches advance the position, so later logins are not limited to
  the first five values or the first 64 positions.
- Compare the same four frame sentinel bytes without cloning/decrypting the
  whole candidate frame. Reject short input and overflowing ranges.
- Export only recovery counters and the highest matched RNG position through
  `recovery_stats`; no origin timestamps or key material are exported.
- Keep the original bounded timestamp fallback and the caller's failure limit.

Regression tests reproduce the old five-value failure at the sixth value,
exercise successful reconnects through position 136, put an incorrect server
candidate before the correct one, and verify bounded/short/corrupt inputs.

Run from the repository root:

```powershell
cargo test -p auto-artifactarium --lib --locked
```

Private game captures and user dispatch keys are not included in this directory.
