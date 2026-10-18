# Security policy

Kopano is research software. It has had one internal security pass (2026-10-18, see [06-security.md](06-security.md)) and **no outside review yet**. Do not rely on it to protect anything you cannot afford to lose.

## Reporting a problem

Please do not open a public issue for a security problem. Use the private **Report a vulnerability** button on the repository's Security tab, or email **support@thabangvision.com** with "Kopano security" in the subject. Say what you found and how to repeat it. We will answer within 7 days and credit you when it is fixed, unless you ask us not to.

## What is protected today

- Only members can sync a project; only approvers can move `main`; each person moves only their own branch.
- Relays and networks cannot read project content (measured: no names or values visible at the relay).
- Every object is checked against its hash; every version, ref and members entry is signed.
- Hostile input does not crash a device (1,000,000 fuzzed inputs, 0 panics) or exhaust its memory.

## Known gaps

See the "Still open" list in [06-security.md](06-security.md). The main ones: no quotas against a member filling a disk, the identity provider is fully trusted, no code signing, no outside review.
