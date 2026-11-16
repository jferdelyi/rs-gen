# data

Training files of the generator. Each `<name>.dat` file becomes a model called `<name>`, loaded by `rs-gen-server` and `rs-gen-exemple` from the `data/` directory of their working directory.

## Format

- UTF-8 text, one entry (word, name, title) per line; `\n` and `\r\n` line endings are both accepted.
- Case is ignored: entries are learned in lowercase, and a generated word is compared to the entries without case.
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

## Binary cache

The first load of `<name>.dat` learns the model and writes `<name>.bin` next to it; the next loads read the `.bin` file. The cache starts with a version header, so a cache written by another version of `rs-gen-core` is rebuilt. `.bin` files are not versioned (`.gitignore`).

Learning needs much more memory than loading: `english.dat` peaks around 1.5 GB while it is learned, against about 650 MB to load the eleven models from their cache. On a small server, generate the `.bin` files on another machine and copy them next to the `.dat` files.

## Adding a model

Add `<name>.dat` to this directory and restart the server: the model appears in `GET /v1/models` and can be weighted with `intensity=<name>:<value>`.
