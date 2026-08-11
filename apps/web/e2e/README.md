# Browser end-to-end

## First run

`first-run.mjs` is the odd one out and worth reading first. Every other suite runs against a seeded
vault and a daemon somebody already started; this one starts from an empty directory and the
shipped binary, which is the path a new user actually takes — and until it existed, the only story
with no test.

```bash
pnpm --filter @summo/web build
cargo build --release -p summo-cli --features bundled
node e2e/first-run.mjs ../../target/release/summo
```

It drives the _bundled_ build, where the daemon serves the interface itself. Three things exist only
on that path and all three have broken at least once: asset routing, injecting the handshake into
the page instead of the URL, and the same-origin write that a browser tags with an `Origin` header.
It makes its own temporary vault and removes it, so it can be run repeatedly.

Drives the whole product the way a person does, in a real browser, with a WAV file standing in for
the microphone. Chromium's `--use-file-for-fake-audio-capture` makes `getUserMedia` return that file,
so this exercises the real path: capture → the worklet's resampling → the WebSocket → the daemon →
voice detection → decoding → events → React → the file on disk.

Every piece has unit tests. This is the only thing that catches them being wired together wrongly.

```bash
# 1. Install the models this needs, once
cargo run --release -p summo-cli -- setup --lang vi --registry ../../../summo-registry

# 2. Start the daemon. --dev lets a page served from this machine reach it; a shipped build
#    refuses that, which is what stops a website reaching your microphone.
cargo run --release -p summo-engine --features models -- --port 8710 --dev

# 3. Serve the built interface
pnpm --filter @summo/web build && python3 -m http.server 8903 -d dist

# 4. Drive it
TOKEN=$(python3 -c "import json;print(json.load(open('$SUMMO_HOME/engine.json'))['token'])")
node e2e/full-flow.mjs http://127.0.0.1:8903/index.html 8710 "$TOKEN" /path/to/recording.wav
```

The recording must be 16 kHz mono 16-bit PCM — Chromium plays it into the fake device as-is.

Screenshots land in `/tmp/shots/`: while recording, in compact mode, and after stopping.
