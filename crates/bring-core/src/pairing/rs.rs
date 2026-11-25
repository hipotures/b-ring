//! Reed–Solomon error correction over GF(2^8) (primitive polynomial `0x11d`,
//! generator `α = 2`, first consecutive root `α^0`).
//!
//! Small and self-contained: the acoustic pairing channel protects a 34-byte
//! frame with 16 parity bytes, so up to 8 corrupted bytes are repaired.
//! Polynomials are big-endian (`p[0]` is the highest-degree coefficient), as
//! in the classic "Reed–Solomon codes for coders" construction.

const PRIM: u16 = 0x11d;

struct Tables {
    exp: [u8; 512],
    log: [u8; 256],
}

const fn tables() -> Tables {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= PRIM;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Tables { exp, log }
}

static GF: Tables = tables();

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        GF.exp[GF.log[a as usize] as usize + GF.log[b as usize] as usize]
    }
}

fn div(a: u8, b: u8) -> u8 {
    debug_assert!(b != 0);
    if a == 0 {
        0
    } else {
        GF.exp[(GF.log[a as usize] as usize + 255 - GF.log[b as usize] as usize) % 255]
    }
}

fn pow(x: u8, power: i32) -> u8 {
    let e = (GF.log[x as usize] as i32 * power).rem_euclid(255);
    GF.exp[e as usize]
}

fn inverse(x: u8) -> u8 {
    GF.exp[255 - GF.log[x as usize] as usize]
}

fn poly_scale(p: &[u8], x: u8) -> Vec<u8> {
    p.iter().map(|&c| mul(c, x)).collect()
}

fn poly_add(p: &[u8], q: &[u8]) -> Vec<u8> {
    let n = p.len().max(q.len());
    let mut r = vec![0u8; n];
    for (i, &c) in p.iter().enumerate() {
        r[i + n - p.len()] = c;
    }
    for (i, &c) in q.iter().enumerate() {
        r[i + n - q.len()] ^= c;
    }
    r
}

fn poly_mul(p: &[u8], q: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; p.len() + q.len() - 1];
    for (j, &b) in q.iter().enumerate() {
        for (i, &a) in p.iter().enumerate() {
            r[i + j] ^= mul(a, b);
        }
    }
    r
}

fn poly_eval(p: &[u8], x: u8) -> u8 {
    p.iter().fold(0, |y, &c| mul(y, x) ^ c)
}

fn generator(nsym: usize) -> Vec<u8> {
    (0..nsym).fold(vec![1], |g, i| poly_mul(&g, &[1, pow(2, i as i32)]))
}

/// Appends `nsym` parity bytes to `msg` (systematic code).
pub fn encode(msg: &[u8], nsym: usize) -> Vec<u8> {
    assert!(msg.len() + nsym <= 255, "RS block longer than 255 bytes");
    let g = generator(nsym);
    let mut out = msg.to_vec();
    out.resize(msg.len() + nsym, 0);
    for i in 0..msg.len() {
        let coef = out[i];
        if coef != 0 {
            for (j, &gj) in g.iter().enumerate().skip(1) {
                out[i + j] ^= mul(gj, coef);
            }
        }
    }
    out[..msg.len()].copy_from_slice(msg);
    out
}

/// Syndromes with a leading zero (so `synd[1..]` are `S_0..S_{nsym-1}`).
fn syndromes(msg: &[u8], nsym: usize) -> Vec<u8> {
    let mut s = vec![0u8; nsym + 1];
    for (i, si) in s.iter_mut().skip(1).enumerate() {
        *si = poly_eval(msg, pow(2, i as i32));
    }
    s
}

/// Berlekamp–Massey: the error locator polynomial.
fn error_locator(synd: &[u8], nsym: usize) -> Option<Vec<u8>> {
    let mut err_loc = vec![1u8];
    let mut old_loc = vec![1u8];
    let shift = synd.len() - nsym;
    for i in 0..nsym {
        let k = i + shift;
        let mut delta = synd[k];
        for j in 1..err_loc.len() {
            delta ^= mul(err_loc[err_loc.len() - 1 - j], synd[k - j]);
        }
        old_loc.push(0);
        if delta != 0 {
            if old_loc.len() > err_loc.len() {
                let new_loc = poly_scale(&old_loc, delta);
                old_loc = poly_scale(&err_loc, inverse(delta));
                err_loc = new_loc;
            }
            err_loc = poly_add(&err_loc, &poly_scale(&old_loc, delta));
        }
    }
    let lead = err_loc.iter().position(|&c| c != 0)?;
    let err_loc = err_loc[lead..].to_vec();
    let errs = err_loc.len() - 1;
    (errs * 2 <= nsym).then_some(err_loc)
}

/// Chien search: positions (indices into `msg`) of the errors.
fn find_errors(err_loc_rev: &[u8], nmess: usize) -> Option<Vec<usize>> {
    let errs = err_loc_rev.len() - 1;
    let pos: Vec<usize> = (0..nmess)
        .filter(|&i| poly_eval(err_loc_rev, pow(2, i as i32)) == 0)
        .map(|i| nmess - 1 - i)
        .collect();
    (pos.len() == errs).then_some(pos)
}

/// Forney: computes and applies the error magnitudes.
fn correct_errata(msg: &[u8], synd: &[u8], err_pos: &[usize]) -> Option<Vec<u8>> {
    let coef_pos: Vec<usize> = err_pos.iter().map(|&p| msg.len() - 1 - p).collect();
    let mut err_loc = vec![1u8];
    for &i in &coef_pos {
        err_loc = poly_mul(&err_loc, &poly_add(&[1], &[pow(2, i as i32), 0]));
    }
    // Error evaluator: (S(x) · Λ(x)) mod x^(ν+1).
    let synd_rev: Vec<u8> = synd.iter().rev().copied().collect();
    let prod = poly_mul(&synd_rev, &err_loc);
    let keep = err_loc.len().min(prod.len());
    let err_eval_rev: Vec<u8> = prod[prod.len() - keep..].to_vec();

    let x: Vec<u8> = coef_pos
        .iter()
        .map(|&p| pow(2, -(255 - p as i32)))
        .collect();
    let mut e = vec![0u8; msg.len()];
    for (i, &xi) in x.iter().enumerate() {
        let xi_inv = inverse(xi);
        let mut prime = 1u8;
        for (j, &xj) in x.iter().enumerate() {
            if j != i {
                prime = mul(prime, 1 ^ mul(xi_inv, xj));
            }
        }
        if prime == 0 {
            return None;
        }
        let y = mul(xi, poly_eval(&err_eval_rev, xi_inv));
        e[err_pos[i]] = div(y, prime);
    }
    Some(poly_add(msg, &e))
}

/// Repairs `block` (message followed by `nsym` parity bytes) and returns the
/// message, or `None` if there are more errors than the code can fix.
pub fn decode(block: &[u8], nsym: usize) -> Option<Vec<u8>> {
    if block.len() <= nsym || block.len() > 255 {
        return None;
    }
    let synd = syndromes(block, nsym);
    if synd.iter().all(|&s| s == 0) {
        return Some(block[..block.len() - nsym].to_vec());
    }
    let err_loc = error_locator(&synd, nsym)?;
    let rev: Vec<u8> = err_loc.iter().rev().copied().collect();
    let err_pos = find_errors(&rev, block.len())?;
    let fixed = correct_errata(block, &synd, &err_pos)?;
    syndromes(&fixed, nsym)
        .iter()
        .all(|&s| s == 0)
        .then(|| fixed[..fixed.len() - nsym].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn clean_roundtrip() {
        let msg: Vec<u8> = (0..34).collect();
        let block = encode(&msg, 16);
        assert_eq!(block.len(), 50);
        assert_eq!(&block[..34], &msg[..]);
        assert_eq!(decode(&block, 16), Some(msg));
    }

    #[test]
    fn known_vector() {
        // From "Reed–Solomon codes for coders" (Wikiversity), nsym = 10.
        let msg = [
            0x40, 0xd2, 0x75, 0x47, 0x76, 0x17, 0x32, 0x06, 0x27, 0x26, 0x96, 0xc6, 0xc6, 0x96,
            0x70, 0xec,
        ];
        let parity = [0xbc, 0x2a, 0x90, 0x13, 0x6b, 0xaf, 0xef, 0xfd, 0x4b, 0xe0];
        assert_eq!(&encode(&msg, 10)[16..], &parity);
    }

    #[test]
    fn too_many_errors_is_not_silently_accepted_as_original() {
        let msg: Vec<u8> = (100..134).collect();
        let mut block = encode(&msg, 16);
        for b in block.iter_mut().take(12) {
            *b ^= 0x5a;
        }
        assert_ne!(decode(&block, 16), Some(msg));
    }

    proptest! {
        #[test]
        fn corrects_up_to_half_the_parity(
            msg in proptest::collection::vec(any::<u8>(), 34),
            errs in proptest::collection::btree_map(0usize..50, 1u8..=255, 0..=8),
        ) {
            let mut block = encode(&msg, 16);
            for (&pos, &flip) in &errs {
                block[pos] ^= flip;
            }
            prop_assert_eq!(decode(&block, 16), Some(msg));
        }
    }
}
