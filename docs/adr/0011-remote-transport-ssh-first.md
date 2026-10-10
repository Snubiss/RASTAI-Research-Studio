# ADR-0011: OpenSSH for remote nodes now; iroh pairing codes in Phase 2

**Status:** Accepted, 2026-10-10

## Context

- The prototype must train on a paired SSH box (§5), and the first remote run should take under 15 minutes from pairing (§6).
- Users already have SSH access to their GPU boxes (Lambda, Vast, RunPod pods, home labs, university machines), with keys, agents, jump hosts and `~/.ssh/config` set up.
- SSH and crypto are never written in-house (§24, §27 category 2).
- Opening a port to the network is off by default (§21).
- NAT traversal and QR pairing matter for boxes you can't SSH into. That isn't the prototype's audience.

## Decision

1. **Pairing:** `rastai node pair --ssh user@host` (command `node.pair`):
   - copies the matching `rastai` binary to the box;
   - generates the node's Ed25519 identity there (node id = `node_<base32 of the public key>`);
   - pins keys on both sides (`<data>/node/paired_daemons.json`, `<data>/node/known_nodes.json`);
   - starts `rastai node serve` as a systemd user unit.
2. **Transport:** the user's OpenSSH client (`nodes.ssh_binary`, default `ssh`), run as a child process by `rastai-transport`. No SSH library and no in-house SSH. One session carries both directions:
   - `-L` forwards a local socket to the remote `node.sock` (`NodeService`, `SandboxService`);
   - `-R` forwards the remote side back to the daemon's `node-hub.sock` (`NodeHubService`).
3. **Handshake:** an integer `protocol_version` (starts at 1), capability flags and a signed challenge in both directions. A mismatch puts the node in `incompatible`, and `node.update` pushes the matching binary.
4. **Liveness:** a `Heartbeat` every 5 s; three missed mark the node `unreachable`. The daemon reconnects with backoff and reconciles with `ListJobs` and `WatchRunEvents` from `acked_sequence` (ADR-0002). Jobs keep running through drops.
5. **Data movement:**
   - Fetch from the source: nodes pull HF weights and datasets directly, with the HF token from `NodeHubService.GetSecret` as a short-lived value. Only private local data is uploaded.
   - Transfers are whole-file BLAKE3 with resumable `PutFile` / `GetFile` from an offset.
   - LoRA adapters always sync back. Full checkpoints stay remote and are fetched on demand.
   - `node.unpair` and teardown wipe unpinned remote data.
6. **Psychology on remote models:** summaries are computed on the node (8.3 KB per token measured, against a ~20 KB budget). Full tensors come only on zoom, through `FetchTensor`, capped at 64 MiB per request (contracts §14 item 13).
7. **Phase 2:** pairing codes and QR over QUIC with iroh 1.x. The node dials out, and the same two gRPC services ride on the QUIC connection. Chunked, deduplicated transfers also come in Phase 2.
8. **Direction:** "nodes always dial out" (§22) holds for iroh. Under SSH the daemon dials, because that's how SSH works (contracts §14 item 5).

**Checked by:** a CI test pairing to `rastai node serve` on localhost through `ssh`; the §32 weeks 11–12 exit criterion (a remote job survives a network drop), run by cutting the SSH session mid-run; a resumable-transfer test that kills a transfer halfway and finishes it with matching BLAKE3.

## Consequences

**Good**
- Works today with every box the user can SSH into, with no provider-specific code.
- Security-critical transport is OpenSSH, which is audited and already trusted by the user.
- No inbound port on either machine.

**Costs**
- Depends on an `ssh` binary on the daemon's machine. Windows (tier 2) ships one, but its socket forwarding must be checked.
- A child-process transport needs supervision: restart on exit, and parsing of `ssh` failures into `RRS-N1xxx` codes.
- No NAT traversal until Phase 2.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A Rust SSH library | Security-critical code we'd have to trust. It wouldn't read the user's SSH config, agent and jump hosts the way OpenSSH does. |
| A TLS port on the node | Opens the network, and needs firewall and NAT setup. |
| iroh first | NAT traversal isn't needed for boxes you can SSH into. More moving parts in the prototype. |
| WireGuard or Tailscale | An external service or system setup the user must do first. |
| Kubernetes or Slurm adapters | Deferred; SkyPilot covers them in Phase 3. |

## Links

- Spec: [§5](../spec/rastai_spec_v5.md#5-definition-of-done), [§22](../spec/rastai_spec_v5.md#22-compute-nodes-and-data), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy)
- Contracts: [§3.2 transports](../architecture/contracts.md#3-processes-and-boundaries), [§4.4 node services](../architecture/contracts.md#4-wire-contracts), [§9.2 node states](../architecture/contracts.md#9-state-machines), [§14](../architecture/contracts.md#14-open-questions) items 5 and 13
- Related: [ADR-0001](0001-process-layout.md), [ADR-0002](0002-durability-worker-event-logs.md), [ADR-0007](0007-local-api-security.md)
