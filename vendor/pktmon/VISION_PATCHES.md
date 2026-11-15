# Vision Overlay local patch of pktmon 0.6.2

Upstream: https://github.com/emmachase/pktmon, crates.io release 0.6.2, MIT.
Original license retained in LICENSE. Selected source files are vendored; no
driver is included or installed. Cargo root applies this via patch.crates-io.

Changes:

- Require realtime backend explicitly for the probe, no silent legacy fallback.
- Enumerate NetworkInterface sources with showHidden=false and attach all or
  one selected component to the session before activation.
- Correct enum-data-source SIZE_T parameters to usize, source detail union
  size/alignment, and WINAPI callbacks/functions to extern system.
- Callback uses shared immutable context and atomic missed-read/write counters,
  validates payload/metadata bounds, reads potentially unaligned metadata,
  copies packet data and tries a bounded 8192-entry channel. No missing-packet
  logging, clock formatting, blocking channel send, or send unwrap in callback.
- Expose callback drops and invalid descriptor counts. Application samples
  counters outside callback and suspends realtime material tracking on loss.

The upstream default/legacy APIs remain for compatibility. Real gameplay
reliability of this patch requires a user-operated elevated Windows test.

References:

- https://learn.microsoft.com/en-us/windows/win32/pktmon/packetmonitor/nf-packetmonitor-packetmonitorcreaterealtimestream
- https://learn.microsoft.com/en-us/windows/win32/pktmon/packetmonitor/nf-packetmonitor-packetmonitorenumdatasources
- https://learn.microsoft.com/en-us/windows/win32/pktmon/packetmonitor/nf-packetmonitor-packetmonitoraddsingledatasourcetosession
