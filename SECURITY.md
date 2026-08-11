# Security

## Reporting

Report privately through [GitHub's advisory form](https://github.com/Techainer/summo-app/security/advisories/new),
not as an issue. You will get a reply within a week.

Please include what an attacker gets, not only what is wrong — the two are often different, and the
gap is what decides how fast this moves.

## What the product claims

These are the promises worth attacking. A hole in any of them is a serious bug:

- **Audio and transcripts do not leave the machine.** Recognition and speaker attribution run
  locally. The only thing that may cross the network is a summarisation or translation prompt, to a
  language model the user configured themselves, and only if they configured one.
- **The daemon binds loopback only.** It listens on `127.0.0.1` and every route requires a token
  written to `~/.summo/engine.json` with the user's permissions. Binding `0.0.0.0` would expose a
  microphone to a network; there is a test that asserts it does not.
- **A web page cannot reach the daemon.** Cross-origin requests are refused unless the daemon was
  started with `--dev`, which no shipped build does.
- **Sync sends nothing readable.** `summo-sync` encrypts with XChaCha20-Poly1305 before anything
  leaves, keys come from a passphrase through Argon2id, and file paths travel as keyed BLAKE3
  digests. A relay operator sees encrypted blobs and their sizes.

## Things that are not vulnerabilities

- **The token is in a file.** It is readable by the user running the daemon, which is the same user
  who could read the vault directly. It is not a secret from that account.
- **The vault is plain Markdown on disk.** That is the point of the product. Anyone with your files
  has your notes; use full-disk encryption, as you would for anything else.
- **A model you installed does something.** Manifests are validated and blobs are checked against
  their sha256, but a model is code that runs on your machine. Non-permissive and gated models are
  fetched from their original host and never mirrored by us.

## Secrets

The CLI never accepts a passphrase or an API key as an argument. Arguments land in shell history and
in `ps` output on a shared machine. Use `SUMMO_SYNC_PASSPHRASE` / `SUMMO_API_KEY`, or let it prompt.
