# rs-gen-core

Library holding all the generation logic: learning character-level n-gram models from text files, caching them, and generating new words from one or several weighted models. It has no networking and no UI code.

## Files

| File | Role |
|---|---|
| `src/lib.rs` | Crate root: exposes `model`, keeps `io` and `hash` internal. |
| `src/io.rs` | File helpers: list the `.dat` files of a directory, build the `.bin` path, name a model after its file. |
| `src/hash.rs` | Fast hasher (multiply-rotate) for the internal tables, faster than SipHash on short keys. |
| `src/model/mod.rs` | Declares the model modules. |
| `src/model/generator.rs` | `Generator`: loads every `.dat` of a directory, adds, removes or renames a model while it is used, draws the models by weight, generates a word, retries when the word already exists. |
| `src/model/prediction_input.rs` | `PredictionInput` and `StartSeed`: generation parameters and model weights. |
| `src/model/multigram_model.rs` | `MultiGramModel`: one model per training file, made of one `NGramModel` per order, its sentences, and its binary cache. `new` loads a model in one call; `from_cache`, `learn_file` (with a `LearnProgress` that counts the lines learned) and `save_cache` are its steps, to tell which one runs. `ModelStats` gives its size and estimated memory; `count_text` gives the same statistics from a text, without learning it. |
| `src/model/ngram_model.rs` | `NGramModel`: the states of one order n, keyed by their prefix of n - 1 characters. |
| `src/model/state.rs` | `State`: the next characters seen after a prefix and their counts, and the weighted draw of the next one. |

## Dependencies

- `serde` and `postcard`: serialization of the binary cache.
- `rand`: random draws (models, n, next character, seeds).
- `num_cpus`: number of processors, hence of partial models learned in parallel.

## Memory

A model keeps a state for every prefix of every order of every word, so the large models hold millions of states (5.4 million for the eleven models of `data/`). The layout is kept small on purpose:

- a state is 24 bytes and stores its first transition inline: 81 % of the states have only one;
- a prefix is a `Box<str>`, without the spare capacity of a `String`;
- tests fail if a state grows above 24 bytes or a table entry above 40 bytes.

Measured on an Apple M1 Pro with the eleven models of `data/`: about 650 MB once loaded, and about 0.04 ms per generated word with every model weighted (`nb_try=100`).

Learning and loading:

- The binary cache is read and written as a stream: loading `english` peaks at 410 MB instead of 473 MB when the cache was read whole.
- `LearnProgress` is updated once per 1 024 lines: on an Apple M1 Pro, learning `english` takes 1.363 s against 1.353 s without it (median of 9 alternated builds, +0.7 %).
- A model is learned as one partial model per processor, merged by moving the states. On ten processors, learning `english` peaks at 924 MB (2.25 times the loaded model); on one processor, the lines are learned straight into the model, about 1.2 to 1.3 times.
- `ModelStats::estimated_bytes` estimates the memory of a loaded model from its states and sentences. It is calibrated on a Linux server (musl allocator), where the eleven models took 581 MB, about 84 bytes per state besides the prefixes; macOS takes about 20 % more.

## Tests

```
cargo test -p rs-gen-core
```

The tests live next to the code (`#[cfg(test)]` modules). Some write temporary training files in the system temporary directory. `count_text` is checked against the learned model on texts chosen for their edge cases (case, accents, markers, empty lines, characters that lowercase to two), and gave the same statistics as the learned model on 15 real training files.
