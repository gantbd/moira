use moira_wire::CodecErrorV1;
use std::fmt::Debug;

pub fn assert_canonical_mutations<T, Decode, Encode>(
    bytes: &[u8],
    mut decode: Decode,
    mut encode: Encode,
) where
    T: Debug,
    Decode: FnMut(&[u8]) -> Result<T, CodecErrorV1>,
    Encode: FnMut(&T) -> Result<Vec<u8>, CodecErrorV1>,
{
    let mut rejected = 0usize;
    let mut inspect = |candidate: Vec<u8>| match decode(&candidate) {
        Ok(value) => assert_eq!(
            encode(&value),
            Ok(candidate),
            "a successfully decoded mutation must already be canonical"
        ),
        Err(_) => rejected += 1,
    };

    for index in 0..bytes.len() {
        for bit in 0..8 {
            let mut candidate = bytes.to_vec();
            candidate[index] ^= 1 << bit;
            inspect(candidate);
        }

        let mut candidate = bytes.to_vec();
        candidate.remove(index);
        inspect(candidate);
    }

    for index in 0..=bytes.len() {
        let mut candidate = bytes.to_vec();
        candidate.insert(index, 0xff);
        inspect(candidate);
    }

    assert!(
        rejected > 0,
        "the mutation corpus must exercise decoder refusal"
    );
}
