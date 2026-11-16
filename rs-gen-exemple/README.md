# rs-gen-exemple

Minimal example of the `rs-gen-core` API: it loads the models of `./data`, sets the generation parameters and the model weights, shows how invalid values are refused, then prints ten generated words.

## Files

| File | Role |
|---|---|
| `src/main.rs` | The example, commented step by step. |

## Dependencies

- `rs-gen-core`.

## Running

From the root of the repository, so that `./data` is found:

```
cargo run -p rs-gen-exemple
```

The example weights the `french`, `fromage`, `ville` and `pokemon` models, which must exist in `data/`.
