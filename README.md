# Tauri + Vanilla TS

## Linux simulation

On Linux, CNC operations use an in-memory mock and do not link FOCAS or connect
to a CNC. Offsets start at zero and reset when the app exits; life/count return zero.
Windows continues to use the real FOCAS client.

```sh
nix develop
pnpm install
pnpm dev:linux
```

After updating `flake.lock`, re-enter `nix develop` before rebuilding so the
application and system graphics drivers use compatible runtime libraries.

`dev:linux` uses X11/XWayland, disables WebKit accelerated compositing and the
DMA-BUF renderer, and selects system EGL vendor manifests and the NixOS graphics
driver directory. It requires X11 or XWayland.

To simulate gauge measurements too, use a valid `src-tauri/config.json` with
`gauge.ip` set to `127.0.0.1` and an available `gauge.port`. This starts the
built-in dummy gauge server. Other gauge IPs still connect via TCP.

This template should help get you started developing with Tauri in vanilla HTML, CSS and Typescript.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
