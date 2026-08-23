# foks-snowpack

`foks-snowpack` implements the canonical MessagePack subset used by FOKS
v0.1.9. It is not a general-purpose MessagePack codec.

The codec accepts:

- null and booleans;
- minimally encoded unsigned and negative integers;
- binary and text byte strings with minimal length headers;
- non-empty positional arrays; and
- zero- or one-entry fixed maps used as Snowpack variants.

It rejects floats, extensions, arbitrary maps, empty arrays, non-minimal
encodings, trailing bytes, excessive nesting, truncated values, and reserved
markers. To remain compatible with the exact v0.1.9 canonicalizer, arrays of
length 16 through 31 are not representable: FOKS rejects `array16` headers up
to and including length 31 even though base MessagePack would allow them.

Text and variant tags are retained as bytes. Schema-generated protocol types
are responsible for applying UTF-8 and known-tag constraints.

Tests include exhaustive marker/boundary cases, checked-in byte fixtures from
the official Go implementation, and QuickCheck properties for round trips,
canonical uniqueness, truncation, trailing bytes, depth, and arbitrary input.
