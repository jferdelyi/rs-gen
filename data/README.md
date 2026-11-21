# data

Training files of the generator. Each `<name>.dat` file becomes a model called `<name>`, loaded by `rs-gen-server` and `rs-gen-exemple` from the `data/` directory of their working directory.

## Format

- UTF-8 text, one entry (word, name, title) per line; `\n` and `\r\n` line endings are both accepted.
- Case is ignored: entries are learned in lowercase, and a generated word is compared to the entries without case.
- Each entry is learned between two markers, U+0002 (start of text) and U+0003 (end of text): every other character, `<` and `>` included, is a letter. A line holding one of the two markers is not learned, and the admin page refuses a file that has one.
- An empty file gives an empty model.

## Models

| File | Entries | Content |
|---|---:|---|
| `elf-female.dat` | 304 | female elf names |
| `elf-male.dat` | 304 | male elf names |
| `english.dat` | 370 105 | English words |
| `female_names.dat` | 3 709 | female first names |
| `french.dat` | 22 735 | French words, with accents |
| `fromage.dat` | 264 | French cheeses, in capitals |
| `male_names.dat` | 982 | male first names |
| `pokemon.dat` | 802 | Pokémon names |
| `surnames.dat` | 9 987 | surnames |
| `us_tv_and_film.dat` | 18 999 | words of US TV shows and films |
| `ville.dat` | 39 201 | French towns, in capitals and without accents |

## Model files

The first load of `<name>.dat` learns the model and writes `<name>.bin` next to it; the next loads read the `.bin` file. The model file starts with a version header, so a model file written by another version of `rs-gen-core`, or a damaged one, is rebuilt. `.bin` files are not versioned (`.gitignore`).

Learning needs more memory than loading. Measured on an Apple M1 Pro: `english.dat` peaks at 924 MB while it is learned on ten processors, against 410 MB to load it from its model file; on one processor, learning peaks at about 1.3 times the loaded model. On a small server, the admin page checks the memory before learning a new model.

## Adding a model

- With the admin page (`simple-web-front/admin.html`): the file is checked (format, memory), learned in the background, then shown, without restarting the server.
- By hand: add `<name>.dat` to this directory and restart the server.

The model then appears in `GET /v1/models` and can be weighted with `intensity=<name>:<value>`. On the server, the admin page keeps the hidden models in `hidden/` and builds the new ones in `building/`: the server reads only the `.dat` files directly in this directory. Next to each model, a `<name>.stats` file keeps its size once it was loaded, and a `<name>.label` file its display name, if the admin gave one (neither is versioned).
