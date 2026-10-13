// The content security policy the daemon sends with every page (rastai-server's web_assets.rs). The browser tests apply
// this copy to their page, as the daemon's header does to the app; csp.test.js keeps the copy equal to the Rust text.

export const DAEMON_CONTENT_SECURITY_POLICY =
  "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'sha256-KBjDeAaGjMz7k+ImE4bb6dA8AiFaIHw8lSI/QlRGHqU='; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
