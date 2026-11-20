# rs-gen-core

Library holding all the generation logic: learning character-level n-gram models from text files, caching them, and generating new words from one or several weighted models. It has no networking and no UI code.

## Files

| File | Role |
|---|---|
| `src/lib.rs` | Crate root: exposes `model`, keeps `io` and `hash` internal. |
| `src/io.rs` | File helpers: list the `.dat` files of a directory, build the `.bin` path, name a model after its file. |
| `src/hash.rs` | Fast hasher (multiply-rotate) for the internal tables, faster than SipHash on short keys. |
| `src/model/mod.rs` | Declares the model modules. |
| `src/model/generator.rs` | `Generator`: loads every `.dat` of a directory, adds or removes a model while it is used, draws the models by weight, generates a word, retries when the word already exists. |
| `src/model/prediction_input.rs` | `PredictionInput` and `StartSeed`: generation parameters and model weights. |
| `src/model/multigram_model.rs` | `MultiGramModel`: one model per training file, made of one `NGramModel` per order, its sentences, and its model file (`.bin`). `new` loads a model in one call; `from_cache`, `learn_file` (with a `LearnProgress` that counts the lines learned) and `save_cache` are its steps, to tell which one runs. `ModelStats` gives its size and estimated memory; `count_text` gives the same statistics from a text, without learning it. |
| `src/model/ngram_model.rs` | `NGramModel`: the states of one order n, keyed by their prefix of n - 1 characters. |
| `src/model/state.rs` | `State`: the next characters seen after a prefix and their counts, and the weighted draw of the next one. |

## Dependencies

- `serde` and `postcard`: serialization of the model files.
- `rand`: random draws (models, n, next character, seeds).

The number of partial models learned in parallel is the number of processors the process may use, read by the standard library (`std::thread::available_parallelism`).

## Memory

A model keeps a state for every prefix of every order of every word, so the large models hold millions of states (5.4 million for the eleven models of `data/`). The layout is kept small on purpose:

- a state is 24 bytes and stores its first transition inline: 81 % of the states have only one;
- a model learns in a hash table, then is frozen once learned or loaded: its prefixes one after the other in a single buffer, its states in 32-byte entries found through an open addressing index (at most 70 % of the slots used). A frozen model that learns again goes back to the hash table first;
- the model file keeps the layout of the hash table (a map from prefix to state): model files written before the frozen table are read as they are, and the other way round;
- tests fail if a state grows above 24 bytes or a frozen entry above 32 bytes.

Measured on an Apple M1 Pro with the eleven models of `data/` (5.9 million states, median of 5 alternated runs):

| Layout | Memory once loaded | Per state, prefix included | Loading | Generation (`nb_try=100`, every model weighted) |
|---|---|---|---|---|
| Hash table | 593 MB | 101 bytes | 0.70 s | 0.041 ms per word |
| Frozen table | 343 MB | 58 bytes | 0.54 s | 0.062 ms per word |

The frozen and the hash table models hold the same states, transitions and sentences: compared on the eleven models, loaded from their model file and learned from their text. A sorted table with a binary search took 294 MB but generated words 6 times slower; an index keeping 32 bits of each hash took 392 MB for 0.046 ms per word.

Learning and loading:

- The model file is read and written as a stream: loading `english` peaks at 410 MB instead of 473 MB when the model file was read whole.
- `LearnProgress` is updated once per 1 024 lines: on an Apple M1 Pro, learning `english` takes 1.363 s against 1.353 s without it (median of 9 alternated builds, +0.7 %).
- A model is learned as one partial model per processor, merged by moving the states. On ten processors, learning `english` peaks at 924 MB (2.25 times the loaded model); on one processor, the lines are learned straight into the model, about 1.2 to 1.3 times.
- The frozen table does not change the learning peak: `english` still peaks at 928 MB on ten processors.
- `ModelStats::estimated_bytes` estimates the memory of a loaded, frozen model from its states and sentences: 46 bytes per state besides the prefixes and the sentences. It is calibrated on a Linux server (musl allocator), where the eleven frozen models took 342 MB, about 44 bytes per state, and `french` alone 45; macOS takes about 20 % more. Right after a model is learned in the process, it takes about 1.4 times this memory until the process restarts: the allocator keeps the memory the hash table freed.
- `ModelStats::estimated_build_bytes` follows the hash table of the learning, 88 bytes per state (581 MB for the eleven models in hash tables, on the same server): 1.5 times on one processor, 2.5 times on several, plus the text. On the server, learning `french` peaked at 26.4 MB with the frozen table, against 25.9 MB before.

## Tests

```
cargo test -p rs-gen-core
```

The tests live next to the code (`#[cfg(test)]` modules). Some write temporary training files in the system temporary directory. `count_text` is checked against the learned model on texts chosen for their edge cases (case, accents, markers, empty lines, characters that lowercase to two), and gave the same statistics as the learned model on 15 real training files.
