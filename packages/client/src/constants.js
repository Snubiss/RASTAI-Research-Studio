// The client's constants (layout frontend-and-plugins.md 3.2). Every other file of the package reads its numbers
// and header names from here.

// How long the event stream waits before each reconnect attempt, in order; after the last it stays at 5 s, so a
// daemon that comes back is found within 5 s (layout 2.2, "offline").
export const RECONNECT_DELAYS_MS = [250, 500, 1000, 2000, 5000];
// After this long without a working stream the status switches from "reconnecting" to "offline".
export const OFFLINE_AFTER_MS = 30_000;
// A stream counts as open at its first event, or once it has stayed up this long after its headers: the daemon
// sends its headers at once and may end the stream right after them (RRS-C0401, or a failure), and such a stream
// must neither report "connected" nor reset the backoff.
export const STREAM_OPEN_GRACE_MS = 500;
// Bigger files open by path through `file.browse`, so nothing is copied (contracts 3.3).
export const MAX_BROWSER_UPLOAD_BYTES = 256 * 1024 * 1024;
// A key in the URL fragment's query (#login=<code>), never the real query string: a fragment never reaches a
// server log or a Referer header (layout 2.1, 2.6).
export const LOGIN_CODE_PARAMETER = "login";
// A fragment with no route of its own (`#login=<code>`) goes to the home section once the code is removed (2.6).
export const HOME_ROUTE = "/home";

// A failed command call is tried once more with the same idempotency key, after this pause, so a command whose
// answer was lost runs at most once (the daemon replays the journaled result).
export const EXECUTE_ATTEMPTS = 2;
export const EXECUTE_RETRY_DELAY_MS = 250;

// The daemon's HTTP surface (contracts 3.3).
export const HEALTH_PATH = "/api/health";
export const LOGIN_PATH = "/api/login";
export const BLOBS_PATH = "/api/blobs";
// What /api/health answers in "status" once the daemon is fully started.
export const HEALTH_STATUS_OK = "ok";

// Request headers (contracts 3.2, 3.3).
export const AUTHORIZATION_HEADER = "Authorization";
export const BEARER_PREFIX = "Bearer ";
export const SESSION_KEY_HEADER = "Rastai-Session-Key";
export const CLIENT_HEADER = "Rastai-Client";
// <name>/<version> of this package, sent on every request so the daemon log says who called.
export const CLIENT_NAME_AND_VERSION = "rastai-client/0.1.0";
export const JSON_CONTENT_TYPE = "application/json";

// The catalog codes the client raises itself (catalog/errors/RRS-U.json).
export const CODE_DAEMON_UNREACHABLE = "RRS-U0001";
export const CODE_LOGIN_CODE_REJECTED = "RRS-U0002";
export const CODE_SIGNED_OUT = "RRS-U0003";
export const CODE_UPLOAD_TOO_LARGE = "RRS-U0006";
// The daemon's codes the client acts on: a gap the bus no longer holds, and the credential refusals that end a
// session (missing or unknown credential, revoked token, used or expired login code).
export const CODE_EVENTS_MISSED = "RRS-C0401";
export const SESSION_REFUSED_CODES = ["RRS-C1002", "RRS-C1006", "RRS-C1007"];
