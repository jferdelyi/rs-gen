# rs-gen

**rs-gen** is a Rust-based N-gram word generator composed of a reusable core library, a REST API server, and a lightweight desktop UI.

The project focuses on:

* clean separation of concerns
* runtime model loading
* flexible generation parameters
* experimentation and tooling

---

## 📦 Workspace structure

rs-gen is a Cargo workspace composed of three crates:

```
rs-gen
├── rs-core      # Core N-gram logic
├── rs-server    # REST API (actix-web)
└── rs-ui        # Desktop UI (egui)
```

---

## 🧠 rs-core

Core library containing all generation logic.

### Features

* N-gram and multi-N-gram models
* Model loading and persistence (.dat files), one sentence per line
* Model merging
* Optional randomness during word generation

After the learning phase, the generator can produce new words based on the learned data.
If a key does not exist in the current N-gram model, the key is reduced by one character, and the generator retries using the (N−1)-gram model. This process continues until a valid transition is found.

When a specific max_n is provided, the generator progressively builds the word using all intermediate N-gram models.
For example, with max_n = 3:
* start with the 2-gram model (empty → 1)
* then use the 3-gram model (empty,1 → 2)
* continue with the 3-gram model (1,2 → 3)

When max_n is set to 0, the entire already generated prefix is used as the key. This allows the generator to freely navigate across all available N-gram sizes. In this mode, generation follow the original training database.

Randomness can influence generation by selecting a different valid N-gram model at any step. This may produce new keys that do not exist in the learned data. To guarantee that generation can always continue while remaining consistent with the training data, a reduce mechanism is applied. During this reduction phase, randomness may optionally be applied as well.

To avoid generating words that already exist in the training database, a maximum number of attempts can be defined.

Finally, generation can be initialized using a seed:
* a custom seed to start from a specific prefix
* or a random seed selected from a specific N-gram model (if the N-gram model index is set to 0, both the model and the initial key are chosen randomly)

### MultiGram model

The **MultiGram** model is a meta-model built by multiple independent N-gram models to improve diversity.

➡️ No networking, no UI — pure logic.

---

## 🌐 rs-server

REST API server built with **actix-web**.

Models can be loaded, replaced, and queried at runtime without restarting the server.

---

### API Endpoints

---

### `GET /v1/generate`

Generate a new word using the currently loaded models.

#### Examples

```
http://127.0.0.1:5000/v1/generate?seed=custom:test&nb_try=100
```

```
http://127.0.0.1:5000/v1/generate?seed=random:2&randomness=0.1&reduce_random=true
```

#### Query parameters

| Parameter       | Description                                |
| --------------- | ------------------------------------------ |
| `max_n`         | Maximum n-gram size (0 = unlimited)        |
| `nb_try`        | Number of attempts to avoid generate word from database      |
| `randomness`    | Randomness factor (0.0 – 1.0), select a random (valid) NGram model considering the already generated part of the word              |
| `reduce_random` | Recompute randomness even during reducing key (results is very random)        |
| `seed`          | `custom:<string>` start with a specific word / `random:<n>` start with a random key from the selected NGram model (0 = random NGram model) / omitted |

---

### `GET /v1/models`

Returns all available models (`.dat` files) in the `data/` directory.

```
http://127.0.0.1:5000/v1/models
```

---

### `GET /v1/loaded_models`

Returns the list of currently loaded models.

```
http://127.0.0.1:5000/v1/loaded_models
```

---

### `PUT /v1/load_models`

Loads a new set of models and replaces the currently loaded ones.

```
http://127.0.0.1:5000/v1/load_models?names=ville,french,fromage
```

| Parameter | Description                                          |
| --------- | ---------------------------------------------------- |
| `names`   | Comma-separated list of model names (without `.dat`) |

---

## 🖥 rs-ui

A simple **egui-based desktop UI** used for:

* selecting models
* tuning generation parameters
* testing the REST API
* visualizing generated output

This UI is intentionally lightweight and developer-oriented.
