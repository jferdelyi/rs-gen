# rs-gen-core

Library holding all the generation logic: learning character-level n-gram models from text files, caching them, and generating new words from one or several weighted models. It has no networking and no UI code.

## Files

| File | Role |
|---|---|
| `src/lib.rs` | Crate root: exposes `model`, keeps `io` and `hash` internal. |
| `src/io.rs` | File helpers: read the lines of a file, list the `.dat` files of a directory, build the `.bin` path. |
| `src/hash.rs` | Fast hasher (multiply-rotate) for the internal tables, faster than SipHash on short keys. |
| `src/model/mod.rs` | Declares the model modules. |
| `src/model/generator.rs` | `Generator`: loads every `.dat` of a directory, draws the models by weight, generates a word, retries when the word already exists. |
| `src/model/prediction_input.rs` | `PredictionInput` and `StartSeed`: generation parameters and model weights. |
| `src/model/multigram_model.rs` | `MultiGramModel`: one model per training file, made of one `NGramModel` per order, its sentences, and its binary cache. |
| `src/model/ngram_model.rs` | `NGramModel`: the states of one order n, keyed by their prefix of n - 1 characters. |
| `src/model/state.rs` | `State`: the next characters seen after a prefix and their counts, and the weighted draw of the next one. |

## Dependencies

- `serde` and `postcard`: serialization of the binary cache.
- `rand`: random draws (models, n, next character, seeds).
- `num_cpus`: number of threads used to learn a model.

## Memory

A model keeps a state for every prefix of every order of every word, so the large models hold millions of states (5.4 million for the eleven models of `data/`). The layout is kept small on purpose:

- a state is 24 bytes and stores its first transition inline: 81 % of the states have only one;
- a prefix is a `Box<str>`, without the spare capacity of a `String`;
- tests fail if a state grows above 24 bytes or a table entry above 40 bytes.

Measured on an Apple M1 Pro with the eleven models of `data/`: about 650 MB once loaded, and about 0.04 ms per generated word with every model weighted (`nb_try=100`).

## Tests

```
cargo test -p rs-gen-core
```

The tests live next to the code (`#[cfg(test)]` modules). Some write temporary training files in the system temporary directory.
