# Verify SSH connections

Run the real SSH integration suite from the repository root:

```sh
scripts/ssh-fixture/run.sh cargo test --locked --test ssh -- --ignored --nocapture
```

Install Docker, `ssh-keygen`, `ssh-agent`, `ssh-add`, OpenSSL, and Python 3. Start the Docker daemon before running the command. Missing prerequisites and startup failures fail the run. The SSH test remains explicitly ignored during the normal test suite because it needs this fixture.

The fixture builds an OpenSSH container and starts PostgreSQL 18 with TLS on a dedicated bridge network. Only the SSH port is published on localhost. The database hostname `database` exists only on that network. `wrong-database` points to the same server but does not match its TLS certificate. The bastion permits forwarding only to these names on port 5432.

Each invocation generates a host key, plain and encrypted Ed25519 keys, an ECDSA P-256 key, RSA PEM and encrypted PKCS#8 keys, a TLS CA, and a database certificate. A disposable `ssh-agent` holds the plain key. The command receives the `QUERI_SSH_*`, `SSH_AUTH_SOCK`, and `SSL_CERT_FILE` variables from `run.sh`. The SSH port stays fixed across the test's container restarts. No developer keys or agent identities are used.

`bash scripts/check-ssh.sh` also runs the native SSH UI checks and credential storage tests against this fixture.

The suite tests host rejection, session and persistent trust, actual host-key replacement, private-key decryption, agent signing, all database TLS modes, hostname mismatch, `pg_sleep` cancellation through a fresh channel, separate tab transactions, and a lost COMMIT outcome. Reconnect checks that a committed write happened once and temporary state was not replayed. Lifecycle checks cancel one or all waiters, invalidate a profile during host verification, and observe the last consumer disconnecting the transport. A human trust prompt remains open for 16 seconds to exceed the network deadline.

The command runs from the fixture directory. Pass any executable and its arguments to `run.sh` for diagnosis. Containers, PostgreSQL volumes, the bridge network, temporary keys, and the agent are removed on exit. The built image remains available for later runs. Docker's bridge still allows a host process to reach a discovered container IP, so tests deliberately use the private DNS alias and assert SSH authentication counts.
