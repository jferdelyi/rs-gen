# rs-gen-server

REST API exposing the generator of `rs-gen-core`, built with actix-web. The endpoints and their parameters are described in the [main README](../README.md#-rs-gen-server).

## Files

| File | Role |
|---|---|
| `src/main.rs` | Loads the models of `./data` at startup, then serves `GET /v1/generate` and `GET /v1/models` on `127.0.0.1:5000`. |

## Dependencies

- `rs-gen-core`: the generator.
- `actix-web` and `actix-cors`: HTTP server and CORS headers.
- `serde`: query parameters.

## Running

The server reads `./data` relative to its working directory and listens on `127.0.0.1:5000` only: put it behind a reverse proxy to expose it.

```
cargo run --release -p rs-gen-server
```

A generation that cannot continue (for instance a `custom:` seed with a character that no model knows) answers `500` with the reason in the body; invalid parameters answer `400`.

## Building for a Linux server

The deployed binary is a static Linux executable, built from macOS with the musl target:

```
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl -p rs-gen-server
```

This needs a musl cross linker, declared in `~/.cargo/config.toml`:

```
[target.x86_64-unknown-linux-musl]
linker = "x86_64-linux-musl-gcc"
```

Copy the binary next to a `data/` directory holding the `.dat` files and their `.bin` cache (see [data/README.md](../data/README.md) for why the cache should be built beforehand on a small server).
