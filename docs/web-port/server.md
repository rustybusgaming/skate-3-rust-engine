# Serving the browser build

`skate-web-server` (`crates/skate-web-server`) serves the browser package that
`tools/web-port/build.sh` writes. It can also serve your own installation to your own
browser, for you only. Multiplayer lobbies and the relay come to the same server in milestone 5.

```
tools/web-port/build.sh                      # target/web/dist
cargo run --release -p skate-web-server      # http://127.0.0.1:8080/
```

| Option | Default | |
|---|---|---|
| `--dist DIR` | `target/web/dist` | The package to serve. |
| `--listen ADDRESS:PORT` | `127.0.0.1:8080` | Where to listen. `0.0.0.0:8080` serves the package to your network (WebGPU still needs HTTPS there, below). |
| `--installation DIR` | off | Also serve your converted installation to your own browser on this machine (below). |

## What it does

- **Smaller downloads.** `build.sh` writes brotli and gzip copies of each file. The server sends
  the smallest one a browser accepts, with the right type, e.g. `application/wasm`, which browsers
  compile while it downloads.
- **Fresh after a rebuild.** Every response says `Cache-Control: no-cache`. Browsers keep their copy
  and ask whether it changed (`Last-Modified`), which costs one small request. So a rebuilt engine
  is never mixed with a cached page of another commit.
- **Basic hardening.** `X-Content-Type-Options: nosniff` on everything. No CORS: other sites cannot
  read from the server.
- **The GPL source link** stays on the page: the server distributes the engine, and the page
  links its exact source.

## Your installation, from the server

Normally each player chooses their installation folder on the page. If you run the server on the
computer that holds your installation, it can serve it to your browser instead: no folder picking,
and nothing kept between visits but your settings.

```
cargo run --release -p skate-web-server -- --installation "C:\Games\Skate 3 Rust Engine"
Serving target/web/dist at http://127.0.0.1:8080/
Your installation C:\Games\Skate 3 Rust Engine (this machine only): http://localhost:8080/#mount=4f1c…
```

Open the printed address. `--installation` takes the package folder, its `data` folder or
`data/installations/<id>`. A package folder brings its `texture-packs` too.

This is the one place game data leaves your disk through a server, so it is for you only, on
this machine:

- **Off unless asked for.** Without `--installation` there is no `/mount` at all.
- **Only on this machine.** With `--installation` the server must listen on a loopback address
  (`127.0.0.1` or `[::1]`), or it refuses to start. Requests must also name this machine as their
  host (`localhost`, `127.0.0.1`, `[::1]`). That stops other websites reaching it through DNS
  rebinding.
- **Not through a proxy.** A request carrying proxy headers (`Forwarded`, `X-Forwarded-For`,
  `CF-Connecting-IP`, …) is refused. A tunnel or reverse proxy in front of the server can serve
  the game, but never your installation.
- **Token required.** Every request needs a token: 32 random bytes made at start, or yours from
  `SKATE_WEB_TOKEN` (at least 32 characters). It travels in the address's `#fragment`, which
  browsers never send to a server. The page then moves it out of the address bar into this tab's
  session.
- **Read-only and inside the folder.** Links are not followed, `..` and absolute paths are
  refused, and responses are `no-store`.

Never point a public server at an installation, and never upload one anywhere.

## Other devices and HTTPS

Browsers only allow WebGPU on secure pages. `http://localhost` counts as secure; any other address
needs HTTPS. Rather than handle certificates itself, the server can sit behind something that
does. Either way, only the game is served this way; your installation stays on the machine that
has it.

**Caddy** (automatic certificates for a domain you own, or `tls internal` on a home network):

```
skate.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

**Cloudflare Tunnel** (no open ports; certificates handled by Cloudflare):

```
cloudflared tunnel --url http://127.0.0.1:8080
```

Static hosts still work for the package alone: GitHub Pages (1 GB per site) serves
`target/web/dist` as it is. Cloudflare Pages caps files at 25 MiB, so the engine
(`skate3rust_bg.wasm`, 72.7 MB) needs R2 or another host there.

## Download size

What a player downloads is the engine, `skate3rust_bg.wasm`. Measured on this commit's build:

| | Raw | gzip -9 | brotli 11 |
|---|---|---|---|
| Before (milestones 2–6) | 132.5 MB | 22.0 MB | — |
| Without the function-name section (now) | **72.7 MB** | 16.9 MB (17.1 MB from Node's gzip, as `build.sh` writes it) | **9.8 MB** |

- **Function names.** The name section was 45 % of the module. It only turns `wasm-function[1234]`
  into Rust names in console stack traces; panic messages keep their text and source location.
  `SKATE_WEB_KEEP_NAMES=1 tools/web-port/build.sh` keeps it for debugging.
- **Brotli** at quality 11 with its largest window takes about three minutes for the engine. Test
  builds (`SKATE_WEB_ALLOW_DIRTY=1`) use quality 9: 11.3 MB in 14 s.

Tried and left out, because they made the download bigger:

| | Raw | gzip -9 | Why not |
|---|---|---|---|
| `wasm-opt -O` (binaryen 123) after `wasm-bindgen` | 63.1 MB | 17.4 MB | Smaller module, but compresses worse: a bigger download, plus another tool to install |
| Thin LTO | 82.9 MB | 17.9 MB | Inlines more across crates |
| Fat LTO, one codegen unit | — | — | Ran out of 15 GB of memory linking after 20 minutes |

Size-tuned optimisation (`opt-level = "s"`) was not tried: the engine runs physics, animation and
audio on the browser's main thread, and speed matters more there than a few more megabytes.
