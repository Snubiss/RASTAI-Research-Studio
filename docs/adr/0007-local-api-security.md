# ADR-0007: Loopback plus socket, a 0600 token file, Host and Origin checks, scoped tokens

**Status:** Accepted, 2026-10-10

**Pending change:** decisions 3 and 4 are superseded in part by ADR-0034, which plan task M1-01 writes: sign-in goes through `rastai open --print-url` and `token.login_code.create`, so Tauri never reads `owner.token`; any loopback Host on any port is accepted, with Origin equal to `http://` plus Host; a too-open `owner.token` refuses the start. See [Notes since acceptance](README.md#notes-since-acceptance) and [plan §10 rows 2, 22 and 36](../plan/implementation-plan.md#10-conflicts-between-docs-and-the-defaults-m1-01-adopts).

## Context

- The daemon is a local HTTP server that can spend money (`gateway.spend`), read conversations and start GPU jobs.
- Any web page the user opens can send requests to `127.0.0.1` (CSRF), and DNS rebinding can make a hostile hostname resolve to loopback.
- Navi, scripts and MCP clients need access, but usually only part of it (chat only, for example).
- Remote GPU boxes must be usable without opening a port to the network (§21).
- §24: in-house protocols get a threat model and fuzzing; crypto, TLS and SSH are never written in-house.

## Decision

1. **Listeners:**
   - `127.0.0.1:47820` (or the next free port in 47820–47829, written to `<runtime>/daemon.port`);
   - `<runtime>/daemon.sock`, in a 0700 directory.
   - Nothing listens on a non-loopback interface unless the user explicitly changes `daemon.listen`.
2. **The owner token** lives at `<config>/auth/owner.token`, mode 0600, and is created on first run.
3. **Browser sessions:**
   - a one-time login code is exchanged at `POST /api/login` for a session cookie (HttpOnly, SameSite=Strict);
   - Tauri reads `owner.token` and passes a one-time code in the start URL;
   - `rastai serve` opens the browser the same way.
   - Tokens never go in `localStorage`.
4. **Host and Origin are checked on every request to the TCP listener**, including `/v1/*` and `/mcp`.
   - Host must be `127.0.0.1:<port>` or `localhost:<port>`.
   - Origin, when present, must match the daemon's own.
   - A failure returns `RRS-C1xxx` with no side effect. This blocks DNS rebinding and cross-site requests.
5. **The CLI** uses `daemon.sock` and is authenticated by peer credentials (`SO_PEERCRED`, same uid).
6. **Scoped tokens** (`token.create`, `token.list`, `token.revoke`):
   - stored as BLAKE3 hashes in the `tokens` table and sent as Bearer tokens;
   - presets: `chat-only` = `chat`; `mcp-default` = `read`, `chat`, `project`;
   - `gateway.spend` and `conversations.read` are never in a preset by default.
7. **The remote UI** goes only through an SSH tunnel to a headless daemon.
8. **Internal sockets** carry their own credentials:
   - `node.sock`: peer credentials plus the handshake token;
   - jobs: `RASTAI_JOB_TOKEN` (32 random bytes);
   - `llama-server`: loopback, a random port and a random `--api-key` per launch.
9. **Third-party plugin iframes** run with `sandbox="allow-scripts"`, no same-origin access and no token. The host bounds their calls by the plugin's `commands:[scopes]` permission.
10. **Assurance:** a written threat model for the gateway, MCP and node protocols before Phase 2; fuzz targets for the HTTP handlers' parsers. Crypto comes only from `rustls`, `ed25519-dalek`, `blake3` and OpenSSH.

**Checked by:** tests that requests with a foreign Host, a foreign Origin, no credentials, or a token missing the scope all fail with the right `RRS-C1xxx` code; a test that `owner.token` is created 0600 and the daemon refuses to start if it's group- or world-readable; an e2e test that a page on another origin can't call `Execute`.

## Consequences

**Good**
- Drive-by attacks from web pages and DNS rebinding are blocked at the door.
- Clients get least privilege, and every call has a principal for the journal.
- No network exposure by default, and remote use rides on the user's existing SSH setup.

**Costs**
- Each client needs a token or a login step. Scripts must call `token.create` once.
- Strict Host checks break unusual setups (custom hostnames, reverse proxies). They need an explicit setting, and that's deliberate.
- Browser-based third-party chat UIs on another origin can't call `/v1/*` directly. Non-browser clients (Navi, scripts, MCP) are unaffected.
- The threat model and fuzzing are ongoing work, not a one-off.

## Alternatives considered

| Alternative | Why not |
|---|---|
| No auth on loopback | Any local process or web page could drive the daemon. |
| TLS on loopback | Certificate pain, and it doesn't stop DNS rebinding on its own. |
| Accounts or OAuth | §24 says no account. Local use needs no identity provider. |
| Tokens in `localStorage` | Readable by any script injected into the page. HttpOnly cookies aren't. |

## Links

- Spec: [§21](../spec/rastai_spec_v5.md#21-command-layer-and-local-api-security), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy)
- Contracts: [§3.2 who talks to whom](../architecture/contracts.md#3-processes-and-boundaries), [§6.3 scopes](../architecture/contracts.md#6-commands), [§8.1 paths](../architecture/contracts.md#8-storage)
- Related: [ADR-0005](0005-command-layer.md), [ADR-0006](0006-ui-transport-http-only.md), [ADR-0011](0011-remote-transport-ssh-first.md), [ADR-0015](0015-plugins-and-licensing.md)
