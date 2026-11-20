# simple-web-front

Static web pages to play with the generator and to manage its models. No build step and no dependency.

- `index.html` lists the models of the API, lets you weight them and tune the parameters, and shows the generated words. Each model shows its display name, if it has one (`GET /v1/labels`). It reads the list again when you come back to the page, and when the server no longer serves a model it asked for (hidden or deleted meanwhile), then generates again.
- `admin.html` is the admin page: login, models with their size and the memory of the server, check and addition of a text file (forced, with a tick, when the model fits only by using swap), hide, show, display name (the name shown to the visitors, the technical name staying the same) and delete, change of the password; a Refresh button reads the memory and the models again, a Back button leads to the generator. A model on its way shows its step (learning with its share, saving, testing, loading) and the list refreshes itself until it ends. Each model is a card, as many per row as the width allows; once a model is added, its check report gives way to a short note, gone when the model is shown.

## Files

| File | Role |
|---|---|
| `index.html` | The generator page: model weights, parameters, generated words. |
| `script.js` | Calls `GET /v1/models` and `GET /v1/generate`, and keeps the settings in the browser. |
| `theme.css` | Every color of both pages, as tokens (`--surface`, `--focus`...), loaded before `style.css`. |
| `style.css` | The style of both pages, with the colors of `theme.css` only. |
| `admin.html` | The admin page. |
| `admin.js` | Calls the admin routes (`/v1/admin/...`) of `rs-gen-server`. |
| `admin.css` | What the admin page adds to `style.css`: model cards, figures, messages, delete button. |
| `icon.svg`, `icon-32.png`, `icon-192.png` | Icon of the pages, in the browser tab and on Android. |
| `apple-touch-icon.png` | Icon of the generator page on the iOS home screen: square, iOS rounds the corners itself. |
| `github-icon.svg`, `reload.svg`, `admin-icon.svg` | Icons of the links and buttons; the faded lock of the generator page leads to the admin page. |

## Accessibility

Both pages aim at WCAG 2.2 AA:
- **Colors**: text reaches a contrast of 4.5:1 on its background, field borders and the focus ring 3:1 on what surrounds them (1.4.3, 1.4.11). Every color is a token of `theme.css`.
- **Keyboard**: the focus is drawn on every control with the `--focus` color (2.4.7). The settings, display name and delete dialogs are native `<dialog>` elements opened with `showModal()`: the page behind stops answering, Escape closes them, and the focus goes back to the button that opened them. The settings dialog opens on its title, which draws no ring: Safari would draw one on the first slider, even after a mouse click. Tab leads to the slider.
- **Messages**: each message of the admin page is a live region (`role="status"` or `role="alert"`) that stays in the accessibility tree while empty, so that its next text is announced (4.1.3). The generator writes a word letter by letter; the whole word is announced apart.
- **Labels**: every field has a visible label, the weights of the models included (3.3.2).
- **Narrow screens**: nothing goes out of a 320 px wide screen (1.4.10).

[`rs-gen-server/tests/web_front.rs`](../rs-gen-server/tests/web_front.rs) checks these rules on the files: `cargo test -p rs-gen-server --test web_front`. The behavior of the dialogs and the layout at 320 px were checked by hand in a Chromium browser. In WebKit, the engine of Safari, were measured: the buttons of the admin header from 320 to 1280 px, and the element the settings dialog opens on; the rest is not tested in Safari.

## API address

`BASE_URL`, at the top of `script.js`, is the address of `rs-gen-server`. Switch to the commented `http://127.0.0.1:5000` line to use a local server started with `cargo run -p rs-gen-server`.

The admin page calls `/generator/api/v1/admin` on its own origin: the session cookie is only sent there. It needs a reverse proxy that serves these files and forwards `/generator/api/` to `rs-gen-server`, as in production. It refuses to run inside a frame.

## Serving

Any static file server works for the generator page, for instance:

```
python3 -m http.server --directory simple-web-front
```

In production, the reverse proxy serves these files and forwards the API path to `rs-gen-server`.
