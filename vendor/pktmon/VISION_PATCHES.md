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

## Windows 10 compatibility backend

- Explicit capability-based auto/realtime/etw selection; no fallback after access or initialization errors.
- Dedicated named ETW session owned by its handle. No orphan-session takeover, no driver unload.
- Idle-only Pktmon control through documented pktmon.exe start/stop commands, NIC/full-packet selection, refusal of existing/unknown global filters.
- Global filters remain untouched. Only complete game UDP packets (22101/22102, IPv4/IPv6) enter the bounded 8192-entry callback queue and raw recorder.
- Owned ETW session buffers bounded at 64 KiB x 256, plus a 4 MiB bounded PktMon memory log discarded after shutdown; EventsLost/RealTimeBuffersLost sampled via ControlTrace QUERY and fed into existing tracking-loss detection.
- Consumer closure/thread completion supervised; stop closes the owned trace and joins its reader before callback context is released.
- Unused upstream utf16string development dependency removed so patched library tests use the workspace lockfile.
- New ETW TDH decoder verified against a saved private ETL (11,501 complete game packets). Forced ETW startup, full UDP delivery, loss counters and graceful shutdown also verified on Windows 11. Live Windows 10 validation still required.

## Wi-Fi link-layer normalization

- PktMon WiFi (type 2) is IEEE 802.11, not Ethernet. Normalize plaintext LLC/SNAP MSDUs to local Ethernet framing before both acquisition backends feed the decoder.
- Support data/QoS, four-address, HT-control and A-MSDU frames with bounded lengths/counts. Do not guess fragmented or encrypted payloads.
- ETW still queues/records game UDP only after normalization. Export payload-type counts, normalized/unhandled WiFi counts and parsing failures without storing credentials or non-game payload samples.
- Regression tests cover IPv4/IPv6, UDP port filtering, normal/QoS/address/HT combinations, aggregation, truncation, fragments and protected-bit offloaded plaintext.
- Tester 02 evidence: ETW startup/stop succeeded with zero reported losses; game UDP only appeared when VPN was enabled. All 506 retained packets were bare IP from the middle of a KCP session, with zero decoded commands. Actual WiFi bytes were not retained by the old type filter; diagnosis needs a new live run.

References:
- https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/pktmonnpik/ne-pktmonnpik-pktmon_packet_type
- https://learn.microsoft.com/en-us/windows-hardware/drivers/network/wdi-transmit-operations-and-offloads
