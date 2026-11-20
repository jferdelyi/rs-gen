# rs-gen-server

REST API exposing the generator of `rs-gen-core`, built with actix-web. The public endpoints and their parameters are described in the [main README](../README.md#-rs-gen-server); the admin routes are described below.

## Files

| File | Role |
|---|---|
| `src/main.rs` | Loads the models of `./data` at startup, serves `GET /v1/generate` and `GET /v1/models` on `127.0.0.1:5000`, registers the admin routes. `rs-gen-server hash-password` prints the hash of a password instead. |
| `src/admin.rs` | Admin routes under `/v1/admin`: login, logout, password change, list, check, add, hide, show, display name, delete. |
| `src/auth.rs` | Password hash (Argon2id) and its change, sessions, limit of the login attempts. |
| `src/catalog.rs` | Model files of `data/` and their status (shown, hidden, learning, saving, testing, loading, failed): moves, deletions, background builds and loads, size files. |
| `src/check.rs` | Checks a text file before it becomes a model: name, content, memory. |
| `src/system.rs` | Memory of the process, of the machine and of the systemd service, read from `/proc` and the cgroup (Linux only). |

## Dependencies

- `rs-gen-core`: the generator.
- `actix-web` and `actix-cors`: HTTP server and CORS headers.
- `serde`: query parameters and JSON bodies.
- `argon2` 0.6.0 (RustCrypto, MIT or Apache-2.0): hash of the admin password.
- `rand` 0.9.2: session tokens.

## Running

The server reads `./data` relative to its working directory and listens on `127.0.0.1:5000` only: put it behind a reverse proxy to expose it.

```
cargo run --release -p rs-gen-server
```

A generation that cannot continue (for instance a `custom:` seed with a character that no model knows) answers `500` with the reason in the body; invalid parameters answer `400`, among them a `nb_try` above 1000.

## Admin

The admin routes manage the models while the server runs. The page that uses them is `simple-web-front/admin.html`.

### Setting the password

There is no default password: the admin routes answer `503` until a password hash is configured. The server keeps only an Argon2id hash of the password, read from the file named by `RSGEN_ADMIN_HASH_FILE`. The password needs at least 12 characters.

```
(umask 077; read -rs P && printf '%s' "$P" | ./rs-gen-server hash-password > admin-hash.new && mv admin-hash.new admin-hash)
RSGEN_ADMIN_HASH_FILE=admin-hash ./rs-gen-server
```

`read -rs` reads the password without showing it; going through `admin-hash.new` keeps the previous hash if the new password is refused. A missing or invalid hash file disables the admin routes only: the public routes keep running.

Once logged in, the admin page changes the password: the server checks the current one, then writes the new hash to the same file (through `admin-hash.new`, readable by its owner only) and closes the other sessions. The file must therefore be writable by the service. If it is not, the change fails and the password stays the same. A forgotten password is replaced by writing a new hash with the command above, as the owner of the file, then restarting the server.

### Routes

Every admin request carries the header `X-Rs-Gen-Admin: 1`. Every route but `login` needs an open session.

| Route | Role | Success |
|---|---|---|
| `POST /v1/admin/login` with `{"password": "..."}` | Opens a session, sent back as a cookie. | `200` |
| `POST /v1/admin/logout` | Closes the session. | `200` |
| `GET /v1/admin/session` | Tells that the session is open. | `200` |
| `POST /v1/admin/password` with `{"current": "...", "new": "..."}` | Changes the password and saves its hash; closes the other sessions. A wrong current password counts as a failed login (`403`, then `429`); a new password refused: `400`; a hash that cannot be saved: `500`, password unchanged. | `200` |
| `GET /v1/admin/models` | Every model with its status (and the share learned while it learns) and size, the memory of the server (available in RAM and with swap), the limits. | `200` |
| `POST /v1/admin/models/check?name=<name>`, the text file as body | Checks the file without saving anything. | `200` with the report |
| `POST /v1/admin/models?name=<name>[&force=true]`, the text file as body | Checks the file again, then builds the model in the background; it is shown once it generates words. `force=true` accepts a model that fits only by using swap. | `202` with the report |
| `POST /v1/admin/models/<name>/hide` | Unloads the model and moves its files to `data/hidden/`. | `200` |
| `POST /v1/admin/models/<name>/show[?force=true]` | Loads the model in the background, if the memory allows it, then moves its files back to `data/`; `force=true` accepts a model that fits only by using swap. A failed load leaves the model hidden, with the reason. | `202` |
| `POST /v1/admin/models/<name>/label` with `{"label": "..."}` | Gives a shown or hidden model a display name, shown to the visitors instead of its name, which stays the same; empty, it removes it. At most 64 characters, on one line: `400` otherwise. `POST /v1/admin/models` takes it too, as `label=<display name>`. | `200` |
| `DELETE /v1/admin/models/<name>` with `{"confirm": "<name>"}` | Deletes the model and its files for good. | `200` |

Without a session: `401`. Without the header: `403`. A failed check: `422` with the report. A model that fits only by using swap, without `force=true`: `422` (with `"forceable": true` for `show`). A name taken, or another model being added or shown: `409`.

### Model files

| Location | Status |
|---|---|
| `data/<name>.dat` and `.bin` | Shown: loaded at startup and used by the generation. |
| `data/hidden/` | Hidden: kept on disk, not loaded, no memory used. |
| `data/building/` | Being built; moved to `data/` once the model generates words. |
| `<name>.stats`, next to the model | Its size, written once it was loaded and moved with it: a hidden model shows its figures after a restart. |
| `<name>.label`, next to the model | Its display name, if any, moved and deleted with it; empty once the display name is removed, so that its date dates the removal. `GET /v1/labels` serves the ones of the shown models. |

A model on its way goes through these statuses, one model at a time:

| Status | Meaning |
|---|---|
| `learning` | Its text file is being learned, with the share of the lines learned (`progress`, from 0 to 1). |
| `saving` | Its model file is being written. |
| `testing` | A new model generates a few words, to check that it works. |
| `loading` | A hidden model is read from its model file; without one, it is learned and saved first. |
| `failed` | The addition failed; the model can be deleted. |

`Generator::new` reads only the files directly in `data/`, so hidden models stay hidden after a restart.

### Check of a new model

- Name: 1 to 64 lowercase letters, digits, `_` or `-`, starting with a letter or a digit, and not taken.
- File: UTF-8, at most 32 MB, at least one word, lines of at most 256 characters (a line of L characters gives about L² / 2 states). Empty lines and lines with `<` or `>` are warnings.
- Memory: the states of the future model are counted without building it, then turned into an estimate of its memory once loaded and while it is built, always keeping 64 MB free. Three cases:
  - it fits in RAM: the model passes;
  - it fits only by using swap: a risk (`risks` in the report), refused unless forced with `force=true`; the generation is then slower;
  - it does not fit even with swap: a problem, refused even when forced, since the service would be killed for lack of memory, restarted, and killed again while loading the same model.

  The memory in RAM is the lowest of `MemAvailable` and of what the systemd limit (`MemoryMax`) leaves. With swap, it adds `SwapFree` and what `MemorySwapMax` leaves. The count stops at the memory with swap. Outside Linux the memory is unknown and not checked.

### Security

- Session cookie `rsgen_admin`: 32 random bytes, `HttpOnly`, `Secure`, `SameSite=Strict`, no `Path` (the browser sends it to the admin routes only), valid 12 hours. Sessions live in memory: a restart closes them.
- Login: 5 attempts without success from one address, then one minute of wait (`429`); one password check at a time.
- Cross-site requests: the admin header needs a CORS preflight, which the server refuses.
- The client address comes from `X-Real-IP`: the reverse proxy must overwrite it with the real address, and the server must stay reachable through the proxy only.
- Every admin action is written to the standard error (`admin: model 'x' hidden`), hence to the journal of the service.

### Reverse proxy and service

- nginx: `proxy_set_header X-Real-IP $remote_addr;` and `client_max_body_size 32m;` on the admin routes (nginx refuses bodies above 1 MB by default).
- systemd, with the hardening of the service: `ReadWritePaths=/opt/generator/data` (the data directory owned by the service user), `StateDirectory=generator` with `StateDirectoryMode=0700` and `Environment=RSGEN_ADMIN_HASH_FILE=/var/lib/generator/admin-hash` (a hash file the service owns, so that the page can change the password), and a `MemoryMax` so that a build that goes beyond the estimate stops the generator only.

## Tests

```
cargo test -p rs-gen-server
```

The tests call the routes through actix-web, on temporary data directories: each admin route without a session or without the header, login, wrong passwords and their limit, the life of a model (check, add, hide, show, delete), names made of paths, the statuses of a model on its way (learning with its share, saving, testing, loading, a failed show that keeps the model hidden, one model at a time), its size kept after a restart, the display names (saved, moved with the model, given at the addition, served by `GET /v1/labels`), the change of password (saved, other sessions closed, wrong current password counted, short new password refused), and with fixed memory figures the forced addition and display of a model that fits only by using swap, and the refusal of one that does not fit even with swap.

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

Copy the binary next to a `data/` directory holding the `.dat` files and their `.bin` model files (see [data/README.md](../data/README.md) for the memory needed to build a model file).
