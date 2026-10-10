# ADR-0031: An in-house sandbox manager over the Docker Engine HTTP API

**Status:** Accepted, 2026-10-10

## Context

- Agent Gym rollouts and Pass@k samples run untrusted, model-written code (§10, §12). Each needs an isolated container with caps on CPU, memory, time and network.
- Users run Docker or Podman. Both speak the Docker Engine HTTP API on a Unix socket.
- Sandboxing runtimes are security-critical (§27 category 2), so the container engine is external. The client that drives it is plain HTTP and is ours to own.
- Orphaned containers leak resources after crashes, and that matters more once rented machines bill per hour (Phase 3).
- Gym training is Linux-only for now (§0.6). MicroVMs (Firecracker) are deferred.

## Decision

1. **`rastai-sandbox`** is an in-house client for the Docker Engine HTTP API, at a pinned API version, over the engine's Unix socket. It runs inside the node. HTTP over the socket goes through one wrapper module in `rastai-sandbox` (hyper, pending its §27 entry per contracts §14 item 1). There's no Docker SDK crate.
2. **Engine choice:** `sandbox.engine_socket`, default auto: the Podman user socket first (`/run/user/<uid>/podman/podman.sock`), then `/var/run/docker.sock`. Any engine that speaks the API works.
3. **Containers:**
   - every container carries the labels `rastai.node_id`, `rastai.run_id` and `rastai.sandbox_id`;
   - caps come from the harness, defaulting to `sandbox.default_caps` (2 CPUs, 4 GiB RAM, 600 s);
   - each node has a sandbox cap (`compute.sandbox_cap`, default 8);
   - network is off unless the harness lists hosts, and then only those hosts;
   - containers never see a Rastai socket or token.
4. **Access goes through the node.** Workers call `WorkerHostService.CreateSandbox` / `ExecInSandbox` / `RemoveSandbox` with their job's sandbox cap. The daemon uses `SandboxService`. Exec streams are proxied.
5. **Orphan sweep:** on node start and on `sandbox.cleanup`, any container with Rastai labels and no live owning job moves `orphaned → removed`. `ListSandboxes` shows orphans.
6. **Step mode** uses `PauseSandbox` / `ResumeSandbox`. The paused sandbox leaves its batch, and its group is recomputed without it or marked partial, per recipe.
7. **Images** are OCI images from the harness, and the "sandbox image" plugin extension point supplies them. Pulls are allowed only from registries the harness names.
8. **Errors** use `RRS-R1xxx`. A missing engine is a clear diagnostic with install guidance, not a crash.

**Checked by:** a Linux CI test against Podman that creates, execs, caps (a memory hog gets OOM-killed inside the cap), pauses and removes a container; a kill -9 test of a node mid-rollout, then restart and sweep, expecting zero labeled containers left; a network test that a container without listed hosts can't resolve or connect.

## Consequences

**Good**
- Works with Docker or Podman, rootless Podman included, with no SDK dependency.
- Labels plus the sweep mean crashes don't leak containers.
- Rollouts run next to the model on the node, and only traces and rewards stream back.

**Costs**
- Container isolation is weaker than a microVM. Model-written code shares the host kernel. That's acceptable on the user's own box with network off. Firecracker comes later.
- The Docker API surface we use (create, start, exec, attach, pause, remove, list, pull) is ours to keep working across engine versions. Pinning the API version limits the drift.
- Gym training doesn't work on macOS and Windows yet.

## Alternatives considered

| Alternative | Why not |
|---|---|
| The `bollard` crate | An external dependency outside the five categories, for a documented HTTP API we can call with our own client. |
| Shelling out to the `docker` CLI | Text parsing, CLI version drift, and it's slower per call. Podman's CLI differs. |
| Firecracker microVMs now | Linux and KVM only, much more setup, and deferred by the spec. |
| Kubernetes jobs | Needs a cluster. Not a desktop tool. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 6, [§10](../spec/rastai_spec_v5.md#10-training-and-evaluation) (Pass@k), [§12](../spec/rastai_spec_v5.md#12-agent-gym), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§2.1 rastai-sandbox](../architecture/contracts.md#2-component-inventory), [§4.4 SandboxService](../architecture/contracts.md#4-wire-contracts), [§9.8 sandbox states](../architecture/contracts.md#9-state-machines), [§11.2 sandbox settings](../architecture/contracts.md#11-configuration)
- Related: [ADR-0001](0001-process-layout.md), [ADR-0014](0014-platform-tiers.md), [ADR-0020](0020-evaluation-first-party-plugin.md)
