use foks_snowpack::{decode, decode_prefix, encode, Value};
use quickcheck::{Arbitrary, Gen, QuickCheck, TestResult};

#[derive(Clone, Debug)]
struct CanonicalValue(Value);

impl Arbitrary for CanonicalValue {
    fn arbitrary(generator: &mut Gen) -> Self {
        Self(value(generator, 0))
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(shrink_value(&self.0).into_iter().map(Self))
    }
}

fn shrink_value(value: &Value) -> Vec<Value> {
    let mut shrunk = Vec::new();
    match value {
        Value::Null | Value::Bool(false) | Value::Variant(None) => {}
        Value::Bool(true) => shrunk.push(Value::Bool(false)),
        Value::Unsigned(value) => {
            shrunk.extend(value.shrink().map(Value::Unsigned));
        }
        Value::Negative(value) => {
            shrunk.extend(
                value
                    .shrink()
                    .filter(|value| *value < 0)
                    .map(Value::Negative),
            );
        }
        Value::Binary(bytes) => {
            shrunk.extend(bytes.shrink().map(Value::Binary));
        }
        Value::Text(bytes) => {
            shrunk.extend(bytes.shrink().map(Value::Text));
        }
        Value::Array(values) => {
            if values.len() > 1 {
                for index in 0..values.len() {
                    let mut candidate = values.clone();
                    candidate.remove(index);
                    shrunk.push(Value::Array(candidate));
                }
            }
            for (index, value) in values.iter().enumerate() {
                for smaller in shrink_value(value) {
                    let mut candidate = values.clone();
                    candidate[index] = smaller;
                    shrunk.push(Value::Array(candidate));
                }
            }
        }
        Value::Variant(Some((tag, inner))) => {
            shrunk.push(Value::Variant(None));
            shrunk.extend(
                tag.shrink()
                    .map(|tag| Value::Variant(Some((tag, Box::new(inner.as_ref().clone()))))),
            );
            shrunk.extend(
                shrink_value(inner)
                    .into_iter()
                    .map(|inner| Value::Variant(Some((tag.clone(), Box::new(inner))))),
            );
        }
    }
    shrunk
}

fn value(generator: &mut Gen, depth: usize) -> Value {
    if depth >= 8 {
        return scalar(generator);
    }
    match usize::arbitrary(generator) % 9 {
        0..=5 => scalar(generator),
        6 => {
            let length = if depth == 0 {
                let boundaries = [1, 2, 7, 15, 16, 17, 31, 32, 33];
                boundaries[usize::arbitrary(generator) % boundaries.len()]
            } else {
                1 + usize::arbitrary(generator) % 3
            };
            Value::Array((0..length).map(|_| value(generator, depth + 1)).collect())
        }
        7 => Value::Variant(None),
        _ => {
            let length = usize::arbitrary(generator) % 8;
            let tag = (0..length).map(|_| u8::arbitrary(generator)).collect();
            Value::Variant(Some((tag, Box::new(value(generator, depth + 1)))))
        }
    }
}

fn scalar(generator: &mut Gen) -> Value {
    match usize::arbitrary(generator) % 7 {
        0 => Value::Null,
        1 => Value::Bool(bool::arbitrary(generator)),
        2 => Value::Unsigned(u64::arbitrary(generator)),
        3 => {
            let value = i64::arbitrary(generator);
            Value::Negative(if value >= 0 {
                -value.saturating_add(1)
            } else {
                value
            })
        }
        4 => Value::Binary(Vec::<u8>::arbitrary(generator)),
        5 => Value::Text(Vec::<u8>::arbitrary(generator)),
        _ => Value::Unsigned(u64::from(u16::arbitrary(generator))),
    }
}

#[test]
fn arbitrary_values_round_trip_byte_exactly() {
    fn property(value: CanonicalValue) -> bool {
        let encoded = encode(&value.0).unwrap();
        decode(&encoded).is_ok_and(|decoded| decoded == value.0)
    }
    QuickCheck::new()
        .tests(2_000)
        .quickcheck(property as fn(CanonicalValue) -> bool);
}

#[test]
fn every_generated_shrink_remains_canonical() {
    fn property(value: CanonicalValue) -> bool {
        value.shrink().all(|candidate| {
            encode(&candidate.0)
                .and_then(|bytes| decode(&bytes).map(|decoded| (bytes, decoded)))
                .is_ok_and(|(_, decoded)| decoded == candidate.0)
        })
    }
    QuickCheck::new()
        .tests(2_000)
        .quickcheck(property as fn(CanonicalValue) -> bool);
}

#[test]
fn every_accepted_byte_string_is_canonical_and_unique() {
    fn property(bytes: Vec<u8>) -> TestResult {
        match decode(&bytes) {
            Ok(value) => {
                TestResult::from_bool(encode(&value).is_ok_and(|encoded| encoded == bytes))
            }
            Err(_) => TestResult::passed(),
        }
    }
    QuickCheck::new()
        .tests(10_000)
        .quickcheck(property as fn(Vec<u8>) -> TestResult);
}

#[test]
fn every_strict_prefix_of_an_encoding_is_rejected() {
    fn property(value: CanonicalValue) -> bool {
        let encoded = encode(&value.0).unwrap();
        (0..encoded.len()).all(|length| decode(&encoded[..length]).is_err())
    }
    QuickCheck::new()
        .tests(1_000)
        .quickcheck(property as fn(CanonicalValue) -> bool);
}

#[test]
fn appending_any_complete_value_is_trailing_junk() {
    fn property(first: CanonicalValue, second: CanonicalValue) -> bool {
        let mut encoded = encode(&first.0).unwrap();
        encoded.extend(encode(&second.0).unwrap());
        decode(&encoded).is_err()
    }
    QuickCheck::new()
        .tests(1_000)
        .quickcheck(property as fn(CanonicalValue, CanonicalValue) -> bool);
}

#[test]
fn prefix_decode_returns_exact_consumed_length_with_arbitrary_suffixes() {
    fn property(value: CanonicalValue, suffix: Vec<u8>) -> bool {
        let encoded = encode(&value.0).unwrap();
        let expected = encoded.len();
        let mut padded = encoded;
        padded.extend_from_slice(&suffix);
        decode_prefix(&padded)
            .is_ok_and(|(decoded, consumed)| decoded == value.0 && consumed == expected)
    }
    QuickCheck::new()
        .tests(2_000)
        .quickcheck(property as fn(CanonicalValue, Vec<u8>) -> bool);
}

#[test]
fn arbitrary_input_never_panics() {
    fn property(bytes: Vec<u8>) -> bool {
        let _ = decode(&bytes);
        let _ = decode_prefix(&bytes);
        true
    }
    QuickCheck::new()
        .tests(10_000)
        .quickcheck(property as fn(Vec<u8>) -> bool);
}

#[test]
fn unsigned_integer_property_covers_the_full_domain() {
    fn property(value: u64) -> bool {
        let expected = Value::Unsigned(value);
        decode(&encode(&expected).unwrap()).is_ok_and(|decoded| decoded == expected)
    }
    QuickCheck::new()
        .tests(5_000)
        .quickcheck(property as fn(u64) -> bool);
}

#[test]
fn negative_integer_property_covers_the_full_domain() {
    fn property(value: i64) -> TestResult {
        if value >= 0 {
            return TestResult::discard();
        }
        let expected = Value::Negative(value);
        TestResult::from_bool(
            decode(&encode(&expected).unwrap()).is_ok_and(|decoded| decoded == expected),
        )
    }
    QuickCheck::new()
        .tests(5_000)
        .max_tests(20_000)
        .quickcheck(property as fn(i64) -> TestResult);
}
