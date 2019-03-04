use blake2::{Blake2b, Digest};
use std::mem;

pub const FEISTEL_ROUNDS: usize = 3;
// 3 rounds is an acceptable value for a pseudo-random permutation,
// see https://github.com/filecoin-project/rust-proofs/issues/425
// (and also https://en.wikipedia.org/wiki/Feistel_cipher#Theoretical_work).

pub type Index = u64;
pub type FeistelHash = Blake2b;

pub type FeistelPrecomputed = (Index, Index, Index);

// Find the minimum number of even bits to represent `num_elements`
// within a `u32` maximum. Returns the left and right masks evenly
// distributed that together add up to that minimum number of bits.
pub fn precompute(num_elements: Index) -> FeistelPrecomputed {
    let mut next_pow4: Index = 4;
    let mut log4 = 1;
    while next_pow4 < num_elements {
        next_pow4 *= 4;
        log4 += 1;
    }

    let left_mask = ((1 << log4) - 1) << log4;
    let right_mask = (1 << log4) - 1;
    let half_bits = log4;

    (left_mask, right_mask, half_bits)
}

// Pseudo-randomly shuffle an input from a starting position to another
// one within the `[0, num_elements)` range using a `key` that will allow
// the reverse operation to take place.
pub fn permute(
    num_elements: Index,
    index: Index,
    keys: &[Index],
    precomputed: FeistelPrecomputed,
) -> Index {
    let mut u = encode(index, keys, precomputed);

    while u >= num_elements {
        u = encode(u, keys, precomputed)
    }
    // Since we are representing `num_elements` using an even number of bits,
    // that can encode many values above it, so keep repeating the operation
    // until we land in the permitted range.

    u
}

// Inverts the `permute` result to its starting value for the same `key`.
pub fn invert_permute(
    num_elements: Index,
    index: Index,
    keys: &[Index],
    precomputed: FeistelPrecomputed,
) -> Index {
    let mut u = decode(index, keys, precomputed);

    while u >= num_elements {
        u = decode(u, keys, precomputed);
    }
    u
}

/// common_setup performs common calculations on inputs shared by encode and decode.
/// Decompress the `precomputed` part of the algorithm into the initial `left` and
/// `right` pieces `(L_0, R_0)` with the `right_mask` and `half_bits` to manipulate
/// them.
fn common_setup(index: Index, precomputed: FeistelPrecomputed) -> (Index, Index, Index, Index) {
    let (left_mask, right_mask, half_bits) = precomputed;

    let left = (index & left_mask) >> half_bits;
    let right = index & right_mask;

    (left, right, right_mask, half_bits)
}

fn encode(index: Index, keys: &[Index], precomputed: FeistelPrecomputed) -> Index {
    let (mut left, mut right, right_mask, half_bits) = common_setup(index, precomputed);

    for key in keys.iter().take(FEISTEL_ROUNDS) {
        let (l, r) = (right, left ^ feistel(right, *key, right_mask));
        left = l;
        right = r;
    }

    (left << half_bits) | right
}

fn decode(index: Index, keys: &[Index], precomputed: FeistelPrecomputed) -> Index {
    let (mut left, mut right, right_mask, half_bits) = common_setup(index, precomputed);

    for i in (0..FEISTEL_ROUNDS).rev() {
        let (l, r) = ((right ^ feistel(left, keys[i], right_mask)), left);
        left = l;
        right = r;
    }

    (left << half_bits) | right
}

const HALF_FEISTEL_BYTES: usize = mem::size_of::<Index>();
const FEISTEL_BYTES: usize = 2 * HALF_FEISTEL_BYTES;

// Round function of the Feistel network: `F(Ri, Ki)`. Joins the `right`
// piece and the `key`, hashes it and returns the lower `u32` part of
// the hash filtered trough the `right_mask`.
#[allow(clippy::needless_range_loop)]
fn feistel(right: Index, key: Index, right_mask: Index) -> Index {
    let mut data: [u8; FEISTEL_BYTES] = [0; FEISTEL_BYTES];

    {
        let mut shift = (HALF_FEISTEL_BYTES - 1) * 8;

        for item in data.iter_mut().take(HALF_FEISTEL_BYTES) {
            *item = (right >> shift) as u8;
            if shift > 0 {
                shift -= 8;
            }
        }
    }

    {
        let mut shift = (HALF_FEISTEL_BYTES - 1) * 8;
        for i in 0..HALF_FEISTEL_BYTES {
            data[i] = (key >> shift) as u8;
            if shift > 0 {
                shift -= 8;
            }
        }
    }

    let hash = FeistelHash::digest(&data);

    let r = (0..HALF_FEISTEL_BYTES).fold(0, |acc, i| acc | Index::from(hash[i * 8]));

    r & right_mask
}

#[cfg(test)]
mod tests {
    use super::*;

    // Some sample n-values which are not powers of four and also don't coincidentally happen to
    // encode/decode correctly.
    const BAD_NS: &[Index] = &[5, 6, 8, 12, 17]; //
                                                 //
    fn encode_decode(n: Index, expect_success: bool) {
        let mut failed = false;
        let precomputed = precompute(n);
        for i in 0..n {
            let p = encode(i, &[1, 2, 3, 4], precomputed);
            let v = decode(p, &[1, 2, 3, 4], precomputed);
            let equal = i == v;
            let in_range = p <= n;
            if expect_success {
                assert!(equal, "failed to permute (n = {})", n);
                assert!(in_range, "output number is too big (n = {})", n);
            } else {
                if !equal || !in_range {
                    failed = true;
                }
            }
        }
        if !expect_success {
            assert!(failed, "expected failure (n = {})", n);
        }
    }

    #[test]
    fn test_feistel_power_of_4() {
        // Our implementation is guaranteed to produce a permutation when input size (number of elements)
        // is a power of our.
        let mut n = 1;

        // Powers of 4 always succeed.
        for _ in 0..4 {
            n *= 4;
            encode_decode(n, true);
        }

        // Some non-power-of 4 also succeed, but here is a selection of examples values showing
        // that this is not guaranteed.
        for i in BAD_NS.iter() {
            encode_decode(*i, false);
        }
    }

    #[test]
    fn test_feistel_on_arbitrary_set() {
        for n in BAD_NS.iter() {
            let precomputed = precompute(*n as Index);
            for i in 0..*n {
                let p = permute(*n, i, &[1, 2, 3, 4], precomputed);
                let v = invert_permute(*n, p, &[1, 2, 3, 4], precomputed);
                // Since every element in the set is reversibly mapped to another element also in the set,
                // this is indeed a permutation.
                assert_eq!(i, v, "failed to permute");
                assert!(p <= *n, "output number is too big");
            }
        }
    }
}
