# fixtures/hf-server

A local fake of the Hugging Face Hub endpoints the node's downloader uses (layout rust-node-gateway-proto.md 7.9). The
tests in `crates/rastai-node/tests/downloads.rs` include `fake_hub.rs` with `#[path]`, so it is test code and never
part of the node.

| Endpoint | Answers |
|---|---|
| `GET /api/models/<repo>` and `GET /api/models/<repo>/revision/<revision>` | the commit and the file list (`siblings`), with `lfs.sha256` for an LFS file and `blobId` for a plain one; `main` and the commit are the known revisions, anything else is 404 |
| `GET /<repo>/resolve/<commit>/<file>` | the file's bytes; a `Range: bytes=N-` gets 206 with the rest, or 200 with the whole file when the fake is set to ignore Range; past the end is 416 |

What the fake can do, per file or per repository:

- **Drop a connection** after N body bytes, once (`drop_after`) or on every request (`drop_every_time`): the client sees
  a short body and retries.
- **Ignore Range** (`ignores_range`): the client must start the partial file over.
- **List a wrong SHA-256** (`listed_sha256`): the client's hash check must refuse the file.
- **Gate a repository** (`gated`): 401 or 403 unless the request carries the granted bearer token. With `resolve_only`
  the listing answers 200 and only the file requests are gated, as the real Hub does for a gated repository.
- **Redirect a file** (`redirect`, one of `Redirect::None`, `ToCdn`, `NoLocation`, `LoopsToItself`): `ToCdn` answers 302
  to a second fake on its own port (built with `FakeHub::start_with_cdn`), which serves the same files under
  `/fake-cdn/`; the CDN fake records its own requests. `NoLocation` answers 302 with no Location header, and
  `LoopsToItself` answers 302 to the file's own address, so a client that follows redirects never ends.
- **Send a wrong Content-Range** (`wrong_content_range`): a 206 whose range starts one byte late.

Every request is recorded (method, path, Range, Authorization), so a test can check what was asked for and what was not.
Names are plain ASCII: the fake does no percent-decoding. The fake listens on 127.0.0.1 only, so no test reaches the
network.
