# D. Proof of concept

Code: `lab/` (Rust workspace: `kopano-core`, `kopano-lab`). Scenario: `lab/scenarios/poc.sh`. Raw logs: `results/e5-poc-run1.log` (file-per-object store) and `results/e5-poc-run2.log` (redb store).

**Real devices:** no. One Windows 11 PC (Ryzen 7 3700X, 16 GB RAM, SATA SSD + 1 TB HDD), three separate OS processes, each with its own data folder and device key. The LAN path is this PC talking to itself through its LAN address. The relay path is real internet: Johannesburg to n0's Frankfurt relay and back. Media used: three Sony ARW files (49 MB each) and one XAVC video (553 MB) from the owner's local footage; media never left this PC (the relay step carried only Kopano metadata, encrypted end to end).

## What was demonstrated

| # | Mission requirement | Result (run 2, redb) | Status |
|---|---|---|---|
| 1 | Two independently identified devices | Alice, Bob (and Carol) each have an ed25519 device key = iroh endpoint id, and a certificate from a lab identity provider | Tested |
| 2 | A shared project | genesis signed by Alice; Bob editor of lane `develop`; Carol not a member | Tested |
| 3 | A versioned image-edit operation | `exposure 0 → 0.35` saved as a signed version in 4.7 to 80 ms | Measured |
| 4 | Sync without resending an unchanged original | Bob's join: 6 objects, 2,211 bytes, 0 media bytes; Bob already held the photo originals, so `fetch-media` reported complete with 0 bytes | Measured |
| 4b | A small edit's cost on the wire | 3 objects, 1,438 bytes, 7 ms on the LAN path; 1,431 bytes, 1.2 s connect + 2.6 s transfer relay-only | Measured |
| 5 | Signed change verification | every received version verified (signature, certificate, project, lane permission) before use; forged versions refused in conformance | Tested |
| 5b | Non-member refused | Carol: "refused: Carol is not a member of this project"; her media request refused by the blob gate | Tested |
| 6 | Temporary disconnection and resumption | 553 MB transfer killed after 1.5 s at 207 to 258 MB; restart fetched only the remaining 295 to 346 MB and verified the whole file | Measured |
| 6b | Peer offline, then back | pull failed after a 30 s timeout with a clear error; after Alice returned the next pull succeeded | Tested (and a UX finding) |
| 7 | Concurrent edits and explicit conflicts | different settings (exposure vs contrast): merged, both kept, 0 clashes. Same setting (0.9 vs -0.4): Bob's value kept, a conflict record with both versions added, synced to Alice | Tested |
| 8 | Integrity and recovery | one byte flipped in Bob's newest version: scan found it, repair removed it, next pull refetched 1 object (1,055 bytes), scan clean | Tested |

## Conformance kit (toy editor)
All seven checks pass: apply(snapshot) is a no-op; save → change → restore returns the exact start; different settings both survive; same setting is a clash with both kept; integrating twice changes nothing; a changed version is refused; items reference media by BLAKE3 only and use declared kinds.

## What the proof of concept does NOT show
- Real separate machines, real home routers, mobile networks, packet loss. The relay path is real internet but both ends were this PC.
- A real app. The toy editor stands in for Light, Photo and Film; the Light connector is designed (its control port was probed live) but not built.
- Production identity. The provider is a lab key, not ThabangVision.
- Multi-device presence, membership changes, revocation, encryption at rest, quotas.
