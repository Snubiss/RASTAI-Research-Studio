# TLS test fixtures

These files are for tests only. `test-server-key.pem` is a private key, committed on purpose: it signs nothing outside
the test suite, and the CA that signed the server certificate is not kept. `test-ca-cert.pem` and
`test-server-cert.pem` are the test CA and the certificate for `localhost` that `http_client.rs` uses. Do not reuse
them anywhere else, and do not treat them as secrets.
