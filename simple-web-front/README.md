# simple-web-front

Static web pages to play with the generator and to manage its models. No build step and no dependency.

- `index.html` lists the models of the API, lets you weight them and tune the parameters, and shows the generated words. It reads the list again when you come back to the page, and when the server no longer serves a model it asked for (hidden or deleted meanwhile), then generates again.
- `admin.html` is the admin page: login, models with their size and the memory of the server, check and addition of a text file (forced, with a tick, when the model fits only by using swap), hide, show and delete, change of the password; a Refresh button reads the memory and the models again, a Back button leads to the generator.

## Files

| File | Role |
|---|---|
| `index.html` | The generator page: model weights, parameters, generated words. |
| `script.js` | Calls `GET /v1/models` and `GET /v1/generate`, and keeps the settings in the browser. |
| `style.css` | The style of both pages. |
| `admin.html` | The admin page. |
| `admin.js` | Calls the admin routes (`/v1/admin/...`) of `rs-gen-server`. |
| `admin.css` | What the admin page adds to `style.css`: table, figures, messages, delete button. |
| `icon.svg`, `icon-32.png`, `icon-192.png` | Icon of the pages, in the browser tab and on Android. |
| `apple-touch-icon.png` | Icon of the generator page on the iOS home screen: square, iOS rounds the corners itself. |
| `github-icon.svg`, `reload.svg`, `admin-icon.svg` | Icons of the links and buttons; the faded lock of the generator page leads to the admin page. |

## API address

`BASE_URL`, at the top of `script.js`, is the address of `rs-gen-server`. Switch to the commented `http://127.0.0.1:5000` line to use a local server started with `cargo run -p rs-gen-server`.

The admin page calls `/generator/api/v1/admin` on its own origin: the session cookie is only sent there. It needs a reverse proxy that serves these files and forwards `/generator/api/` to `rs-gen-server`, as in production. It refuses to run inside a frame.

## Serving

Any static file server works for the generator page, for instance:

```
python3 -m http.server --directory simple-web-front
```

In production, the reverse proxy serves these files and forwards the API path to `rs-gen-server`.
