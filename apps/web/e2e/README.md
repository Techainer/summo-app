# Browser end-to-end

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
