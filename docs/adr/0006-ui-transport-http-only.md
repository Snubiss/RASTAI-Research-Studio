# ADR-0006: The UI talks to the daemon over HTTP only; Tauri is just a shell

**Status:** Accepted, 2026-10-10. Decision 4 superseded in part by [ADR-0034](0034-login-codes-loopback-host-owner-token.md), 2026-10-10.

**Superseded in part:** decision 4's sign-in and file-association steps. The shell makes no HTTP request: it starts or finds the daemon only by running `rastai open --print-url` and loads the URL it prints, and it passes file associations to the web app as a `#/open?path=` URL fragment; the web app then runs `file.open` (ADR-0034 decision 2). The window and the updater stand as decided.

## Context

- The same UI must work in three places: the Tauri desktop window, a plain browser, and against a headless daemon on a GPU box over an SSH tunnel (§16).
- Tauri on Linux uses WebKitGTK. The spike found it works (WebGL2 at the refresh cap), but it animates at ~60 Hz on a 144 Hz monitor and hides its GPU name from pages (spike 06). A browser fallback should always be one URL away.
- Tauri's own IPC (commands, events) only exists inside Tauri. Any feature built on it would need a second path for the browser and headless mode.
- Types must not be defined twice (ADR-0004).

## Decision

1. **`apps/web` reaches the daemon only through `@rastai/client`, over HTTP.**
   - Connect (protobuf binary by default, JSON accepted) for `CommandService` and `EventService` on `127.0.0.1:47820` (falling back to the next free port in 47820–47829);
   - plain GETs for blobs (`/api/blobs/<hash>`, with HTTP Range) and static files.
   - No JavaScript package reads files or opens any other socket (contracts §2.4 rule 4).
2. **The daemon serves the web app** from the binary at `/`. Plugin UI assets are served at `/plugins/<plugin_id>/<version>/ui/*`.
3. **Events** come from `EventService.Subscribe(after_bus_sequence, event_types, subject_ids)`.
   - The daemon keeps the last 10,000 bus events (`daemon.bus_buffer_events`).
   - A client that reconnects with a sequence still in the buffer gets the gap replayed. Otherwise it gets `RRS-C0401` and refetches state with `*.list` / `*.get` commands.
4. **The Tauri 2 shell (`apps/desktop`) does exactly four things:**
   - the window;
   - file associations (passed on as a `file.open` command over HTTP); *(superseded: a `#/open?path=` fragment, ADR-0034)*
   - the updater;
   - starting `rastai serve` and opening the UI with a one-time login code (ADR-0007). *(superseded: the shell only runs `rastai open --print-url`, ADR-0034)*
   It defines no Tauri commands for app features and never touches app data.
5. **A plain browser and headless mode are full clients** of the same build. `rastai serve --headless` runs the API and MCP with no window.
6. **Live packets use the same stream.** At 8.3 KB × ~150 tokens/s that is about 1.3 MB/s. Move the live stream to a WebSocket only if measurement in WebKitGTK shows stalls (contracts §14 item 11).

*Note since acceptance (2026-10-10):*
- *"TS package" reads "JavaScript package" (ADR-0036).*
- *`apps/web` holds no domain logic: anything stateful or computed is a command, and the UI renders, routes and keeps view state only (contracts §2.4 rule 9). M1-49's three-surfaces test grows each milestone, and M10-E02 adds an API-only run of the core loop.*
- *Cross-origin browser clients are refused in v1; the Phase 2 routes are daemon-hosted alternative UIs on the daemon's own origin and an origin allowlist with pairing (ADR-0034 decision 7).*
- *`EventService` is binary-only Connect in the prototype; a generated proto-JSON codec for it comes in M9 if needed, else Phase 2 (contracts §3.2, §4).*
- *The other decisions are unchanged.*

**Checked by:** the Playwright e2e suite runs against the web build in a plain browser, not the Tauri window, so anything that only works in Tauri fails CI. A grep check fails if `apps/web` or any package imports `@tauri-apps/*`.

## Consequences

**Good**
- One client code path for desktop, browser and remote. Headless GPU boxes need nothing extra.
- If WebKitGTK misbehaves on some system, the same UI works in Chrome or Firefox.
- Every UI action is a command, so the UI can do nothing the CLI and MCP can't.

**Costs**
- The desktop app has no native shortcuts into the filesystem. File access goes through the daemon (`PUT /api/blobs` for drag-drop, `file.open` for paths).
- Live streaming goes through HTTP/1.1 framing rather than shared memory. That's measured as acceptable at 1.3 MB/s, but it's not free.
- A local HTTP server is an attack surface. ADR-0007 handles it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Tauri IPC for app features | Ties features to Tauri. The browser and headless mode would need a second path. |
| Electron | Ships its own Chromium (large), and isn't on the allowlist. HTTP-only already hedges WebKitGTK. |
| Hand-written REST + SSE | Types would be written twice. Connect is generated from the same proto as everything else. |
| gRPC-web | Needs a proxy or trailer support. Connect runs over plain HTTP/1.1. |
| WebSocket with a custom JSON protocol | Re-invents typed RPC, and loses generated clients. |

## Links

- Spec: [§4](../spec/rastai_spec_v5.md#4-audience-and-platforms), [§16](../spec/rastai_spec_v5.md#16-architecture-overview), [§21](../spec/rastai_spec_v5.md#21-command-layer-and-local-api-security)
- Contracts: [§3.3 daemon HTTP surface](../architecture/contracts.md#3-processes-and-boundaries), [§7.2 bus events](../architecture/contracts.md#7-events), [§14](../architecture/contracts.md#14-open-questions) items 9 and 11
- Spike: [06 canvas in webviews](../../spike/results/06_canvas_webviews.md)
- Related: [ADR-0007](0007-local-api-security.md), [ADR-0014](0014-platform-tiers.md), [ADR-0019](0019-in-house-webgl2-canvas.md), [ADR-0034](0034-login-codes-loopback-host-owner-token.md), [ADR-0036](0036-frontend-plain-javascript.md)
