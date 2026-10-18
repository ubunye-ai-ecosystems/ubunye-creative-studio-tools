# E26 security pass, raw results (2026-10-18, dev box, Windows 11, release build)

## Attacks (tests/attacks.rs)
Before the fixes: a1 FAILED, a2 FAILED, a3 FAILED, a4 ok (3 of 4 attacks worked).
After: 4 passed, 0 failed.

## Fuzzer (tests/fuzz_inputs.rs, FUZZ_ITERS=1000000, 434.65 s)
{"accepted_forgeries":0,"bomb_compressed_bytes":203699,"bomb_refused_when_declared":true,"bomb_refused_when_size_lies":true,"iterations":1000000,"mutated_payloads_decoded":21334,"panics":{}}

## Relay view (scenarios/e25_relay_view.py, local iroh-relay 1.3.0, relay-only sync of 200 photos + 1 edit)
{"pull_ok": true, "pull_path": "Relay(http://127.0.0.1:4000/)", "relay_bytes_seen": 267791, "private_found_in_relay_traffic": {"project name": false, "photo file name": false, "edit value": false, "person name": false, "account id": false, "device label": false, "setting name": false}, "visible_by_design": {"Alice endpoint id (hex)": true}}
Bob's copy held the edit value afterwards (checked with `peer status`).

## Stranger flood (peer flood, 50-photo project, serve on this PC)
Before (64-slot cap, 10 s handshake, newcomer refused): stranger held 64 slots; member pull: "Error: closed by peer: busy".
After (16-slot waiting room, oldest evicted, 5 s handshake), two attackers for 40 s:
{"alive_at_end":9,"event":"flood","opened":251,"refused":0,"seconds":40}
{"alive_at_end":6,"event":"flood","opened":217,"refused":0,"seconds":40}
member pull 1..5: all "fast-forward, 50 item(s) applied", 1163 / 1177 / 1184 / 1204 / 1310 ms

## Device key at rest (peer protect-key)
device.key before: 32 raw bytes. After: "keystore:device-3b61b2cd6f7139f6" (Windows Credential Manager entry device-3b61b2cd6f7139f6.kopano).
Pull with the protected key: fast-forward, 52 objects. Copied folder after the entry was removed:
"Error: device key device-3b61b2cd6f7139f6 not in this user's key store (No matching entry found in secure storage); was the folder copied from another user or machine?"

## Dependencies (lab/deny.toml)
cargo audit: 1338 advisories loaded, 464 crates, 0 vulnerabilities; unmaintained: atomic-polyfill, paste (other targets only), proc-macro-error (via iroh-blobs).
cargo deny check licenses advisories: advisories ok, licenses ok (Unlicense allowed for 3 wasm-only crates under iroh-relay).

## Workflow simulator after the fixes (owner seals main after every members change; cheats also backdate and lie about seq)
200 seeds x 150 ops: passed 200, failures {}, Stats { edits: 6801, approvals: 1366, clash_decisions: 1069, catch_ups: 1490, syncs: 19529, cut_syncs: 1000, member_changes: 661, cheats: 1969, forks_reoffered: 43 }

500 seeds x 300 ops (3653 s): passed 500, failures {}, Stats { edits: 38285, approvals: 6367, clash_decisions: 7320, catch_ups: 6362, syncs: 64388, cut_syncs: 5092, member_changes: 3438, cheats: 10078, forks_reoffered: 634 }
