# G. Security: threat model and attack results (2026-10-18)

This is the security pass done before the protocol goes public. Method: name every attacker, try the attack in code, fix what worked, and run the attack again. Every claim below points at a test or a measured run. What is still open is listed at the end, plainly.

## What we protect

1. **Who can change the project.** Only the owner and the approvers the owner names can move `main`. Each person moves only their own branch, and only in the lanes they were given.
2. **What the project contains.** Edits, notes and media stay with members. Nobody in between can read them.
3. **Device identity.** A device key is the device. Whoever holds it can act as that device.
4. **Being able to work.** A stranger must not be able to stop members from syncing.

## Who might attack, and what happens

| # | Attacker | Attack | Answer | Evidence | Status |
|---|---|---|---|---|---|
| 1 | Stranger on the internet | Connect and ask for the project | Must show a certificate signed by the project's identity provider for a current member, bound to the connection's key | `a4_removed_member_and_foreign_refs`, E5 step 7 | Fixed before, tested |
| 2 | Stranger | Open many silent connections so members cannot get in | Strangers wait in a room of 16 slots; when full, the **oldest** waiter is pushed out; 5 s to show a certificate; members have their own 64 slots | E26 flood: 468 silent connections from 2 attackers in 40 s, **5 of 5 member syncs got through** (1.2 s each). Before the fix: member refused "busy" | **Fixed in this pass** |
| 3 | Stranger | Send a message slowly to hold memory | Every message has a deadline (30 s plus 1 s per 16 KiB); 64 MiB cap per message | code review, `read_frame` | **Fixed in this pass** |
| 4 | Relay operator (n0, ours, anyone in between) | Read the work | Everything is inside QUIC TLS between the two devices | E25: a full sync recorded at the relay (268 KB). Project name, photo names, person name, account id, setting names and the edit value: **none found** | Tested |
| 5 | Relay operator | Learn who works with whom | Sees device endpoint ids (public keys), IP addresses, timing and sizes | E25: Alice's endpoint id **is** visible | Accepted, documented |
| 6 | Relay or network | Change bytes in transit | QUIC authenticates every packet; every object is checked against its BLAKE3 hash | conformance `tamper`, E5 step 12 | Tested |
| 7 | Member | Send garbage, truncated or hostile data | Decoders are bounds-checked and never panic | `fuzz_inputs`: **1,000,000** mutated payloads, node deltas and signed objects: **0 panics** | Tested |
| 8 | Member | Deflate bomb: tiny message that unpacks to gigabytes | Declared size capped at 64 MiB and enforced while unpacking | `fuzz_inputs`: 200 MB of zeros in 204 KB refused, honest and lying size both. Before the fix: up to 1 GiB reserved | **Fixed in this pass** |
| 9 | Member | Change a signed version, ref or members entry | Signatures over canonical JSON; content addressing | `fuzz_inputs`: **0 forgeries accepted** in 1,000,000 tries | Tested |
| 10 | Member | Edit a lane they were not given, move `main`, move someone else's branch | Every device recomputes the rights from data | sim_workflow: thousands of cheats per run, all rejected (W3) | Tested |
| 11 | Approver | Claim a huge sequence number to win every fork | An update must be its predecessor's seq + 1 | `a2_seq_must_follow_previous` (worked before the fix) | **Fixed in this pass** |
| 12 | Demoted approver | Keep approving into `main` by citing the members list from before the demotion | `main` updates must cite at least what the previous one cited; the owner **seals** `main` after every members change; a signer who is no longer an approver loses every fork | `a1_demoted_approver_cannot_take_main` (worked before the fix) | **Fixed in this pass** |
| 13 | Demoted or removed admin | Sign an old-looking members entry and grind its id so it sorts before the demotion, adding a sock-puppet admin | Concurrent demotion wins: an admin's entry that the demotion did not see is dropped | `a3_demoted_admin_cannot_plant_a_puppet` (worked before the fix: the puppet stayed admin) | **Fixed in this pass** |
| 14 | Removed member | Keep pushing their branch | Branch updates are checked against the current members | `a4_removed_member_and_foreign_refs`, sim W3 | Tested |
| 15 | Anyone | Replay a ref from another project, or a ref signed by another project's owner | Project id is signed into every object | `a4_removed_member_and_foreign_refs` | Tested |
| 16 | Thief with a copy of the data folder | Act as the device | On Windows and macOS the key moves into the OS key store (`peer protect-key`); the folder only names it | Real run: protected key synced normally; the copied folder fails with "not in this user's key store" | **Fixed in this pass** (Linux: see open items) |
| 17 | Thief with the laptop itself | Act as the device | Owner blocks that device (signed `BlockDevice`); the person gets a new device certificate from the identity provider and keeps their account | sim_workflow (blocked devices' work rejected) | Tested |
| 18 | Supply chain | A known-vulnerable or wrongly licensed dependency | `cargo audit` and `cargo deny` with `lab/deny.toml` | 464 crates: **0 vulnerabilities**, licences all compatible with MIT OR Apache-2.0; 3 crates flagged unmaintained (not vulnerable, all via iroh) | Tested; add to CI before release |

The whole v2 simulator was rerun after these fixes, including smarter cheats that backdate and lie about sequence numbers (see `results/e26-security.md`).

## What the fixes cost honest users

- **An admin's offline change can be dropped** if the owner demotes that admin at the same time without having seen the change. The owner can redo it. This is the usual "removal wins" rule.
- **An approval made at the same moment as the approver's demotion** loses its fork. Its work is not lost: it is offered for approval again.
- Clocks play no part in any rule, so a phone with the wrong date works normally.

## Still open (not fixed, in order of importance)

1. **The seal window.** If an honest approver builds on a demoted approver's update before the owner's seal reaches them, that update stays in `main`. The app must show it as "approved by a former approver" for the owner to review. Not built yet.
2. **Members can fill a disk.** Objects from a member are stored (hash checked) before the versions that use them are verified. There are no quotas. A stranger cannot do this; a member can.
3. **The identity provider is fully trusted.** Whoever controls its signing key can issue a certificate for any account, including the owner's. A project can list several providers, but one bad provider is enough. Planned: owner-pinned device keys for admins.
4. **A rogue admin can block the owner's devices.** The owner gets a new device certificate and removes that admin, but it is a nuisance. Fix: only the owner may block the owner's devices.
5. **Certificate expiry is not a security boundary.** Signed times are chosen by the signer, so an expired device can claim an old time. Blocking a device is the real control.
6. **Keys and data on the device.** On Linux the key stays in the file (readable by its owner only). On every system, malware running as the user can use the key. Project data on disk is not encrypted: people should use BitLocker or FileVault.
7. **Two admins demoting each other at the same moment:** the order is decided by object ids, which a signer can influence. The owner can always correct it.
8. **What the relay learns** (item 5): endpoint ids, IP addresses, timing, sizes. Running our own relay keeps this inside the project's community.
9. **Not done at all:** code signing of the apps; an outside security review; coverage-guided fuzzing (`cargo fuzz`, needs Linux CI). The fuzzer here is random mutation.

The lab's remote test helper (`agent`) is for testing only and must never ship in a release.

## How to rerun everything

```
cargo test --release -p kopano-lab --test attacks
FUZZ_ITERS=1000000 cargo test --release -p kopano-lab --test fuzz_inputs -- --nocapture
SIM_SEEDS=500 SIM_OPS=300 cargo test --release -p kopano-lab --test sim_workflow -- --nocapture
python scenarios/e25_relay_view.py <peer.exe> <iroh-relay.exe> <work dir>
peer flood --from <addr.json> --n 200 --hold-s 40     # while a member pulls
cargo audit && cargo deny check licenses advisories
```
