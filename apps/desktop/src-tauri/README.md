# The desktop shell

Excluded from the cargo workspace on purpose. Building it needs platform webview libraries —
`webkit2gtk-4.1` on Linux, WebView2 on Windows, WebKit on macOS — and making the core crates depend
on those would mean `cargo test` could not run on a headless server or a plain CI runner.

```bash
pnpm install
pnpm build              # the frontend, which the shell embeds
cargo tauri dev         # from this directory
```

The shell spawns `summo-engine` as a sidecar and talks to it over loopback. Copy or symlink the
built engine binary to `binaries/summo-engine-$TARGET_TRIPLE` before bundling.

**Not yet verified on a real desktop.** This configuration was written and type-checked, but no
machine in the development environment has a display server, so the window, tray icon and global
shortcut have not been exercised.
