These public test credentials are exclusively for in-memory rustls tests.
The EC P-256 leaf is issued to `localhost` by `Rabbit Test Root`. The unencrypted
PKCS#8 key is intentionally checked in for the test server; never deploy it.
Tests use a fixed verification time inside the certificates' 2026–2126 validity
period. No system trust store or external network is used.
