# Inside foks-rs

A book about the algorithms, data structures, and engineering techniques used
by [foks-rs](../foks-rs), a Rust implementation of FOKS, the Federated Open
Key Store. It is written for readers with a modest computer science background
and cites the standards and papers behind each technique.

The book is a [Quarto](https://quarto.org/docs/books/) book project. Chapters
are the `.qmd` files in this directory; the outline is in `_quarto.yml`.

```sh
quarto render          # renders HTML to _book/
quarto preview         # live preview in a browser
```

`docs/superpowers/` holds the design spec and writing plan used to produce
the book, and `docs/research/` the research notes it was written from.
