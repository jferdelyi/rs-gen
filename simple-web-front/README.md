# simple-web-front

Static web page to play with the generator: it lists the models of the API, lets you weight them and tune the parameters, and shows the generated words. No build step and no dependency.

## Files

| File | Role |
|---|---|
| `index.html` | The page: model weights, parameters, generated words. |
| `script.js` | Calls `GET /v1/models` and `GET /v1/generate`, and keeps the settings in the browser. |
| `style.css` | The page style. |
| `icon-192.png` | Icon of the page. |
| `github-icon.svg`, `reload.svg` | Icons of the links and buttons. |

## API address

`BASE_URL`, at the top of `script.js`, is the address of `rs-gen-server`. Switch to the commented `http://127.0.0.1:5000` line to use a local server started with `cargo run -p rs-gen-server`.

## Serving

Any static file server works, for instance:

```
python3 -m http.server --directory simple-web-front
```

In production, the reverse proxy serves these files and forwards the API path to `rs-gen-server`.
