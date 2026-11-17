//! Per-cell reference (independent of the contract's bit tricks), patterns, rendering.
//! Bit index = row * 16 + col, same as `examples/life/src/board.fe`.
use revm::primitives::U256;

pub const N: usize = 16;
pub type Grid = [[bool; N]; N];

pub fn unpack(b: U256) -> Grid {
    let mut g = [[false; N]; N];
    for (r, row) in g.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = b.bit(r * N + c);
        }
    }
    g
}

pub fn pack(g: &Grid) -> U256 {
    let mut b = U256::ZERO;
    for r in 0..N {
        for c in 0..N {
            if g[r][c] {
                b.set_bit(r * N + c, true);
            }
        }
    }
    b
}

/// Plain per-cell B3/S23 on a torus.
pub fn ref_step(g: &Grid) -> Grid {
    let mut out = [[false; N]; N];
    for r in 0..N {
        for c in 0..N {
            let mut n = 0;
            for dr in [N - 1, 0, 1] {
                for dc in [N - 1, 0, 1] {
                    if (dr, dc) != (0, 0) && g[(r + dr) % N][(c + dc) % N] {
                        n += 1;
                    }
                }
            }
            out[r][c] = n == 3 || (g[r][c] && n == 2);
        }
    }
    out
}

pub fn ref_step_packed(b: U256) -> U256 {
    pack(&ref_step(&unpack(b)))
}

fn from_cells(cells: &[(usize, usize)], dr: usize, dc: usize) -> U256 {
    let mut g = [[false; N]; N];
    for (r, c) in cells {
        g[(r + dr) % N][(c + dc) % N] = true;
    }
    pack(&g)
}

/// xorshift64* so `random` is reproducible from `--seed` without extra dependencies.
pub fn random_board(seed: u64) -> U256 {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut g = [[false; N]; N];
    for row in g.iter_mut() {
        for cell in row.iter_mut() {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            *cell = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 63) == 1;
        }
    }
    pack(&g)
}

pub fn pattern(name: &str, seed: u64) -> Option<U256> {
    match name {
        "glider" => Some(from_cells(&[(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)], 1, 1)),
        "blinker" => Some(from_cells(&[(0, 0), (0, 1), (0, 2)], 7, 6)),
        "rpentomino" => Some(from_cells(&[(0, 1), (0, 2), (1, 0), (1, 1), (2, 1)], 7, 7)),
        "random" => Some(random_board(seed)),
        _ => None,
    }
}

pub fn render(b: U256, color: bool) -> String {
    let g = unpack(b);
    let mut s = String::new();
    for row in g.iter() {
        for &cell in row.iter() {
            s.push_str(match (cell, color) { (true, true) => "\x1b[32m##\x1b[0m", (false, true) => "\x1b[90m. \x1b[0m", (true, false) => "##", (false, false) => ". " });
        }
        s.push('\n');
    }
    s
}

pub fn alive(b: U256) -> usize {
    b.count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut b: U256, n: usize) -> U256 {
        for _ in 0..n {
            b = ref_step_packed(b);
        }
        b
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let b = random_board(3);
        assert_eq!(pack(&unpack(b)), b);
        // bit index = row * 16 + col
        let mut g = [[false; N]; N];
        g[2][5] = true;
        assert_eq!(pack(&g), U256::from(1u64) << (2 * 16 + 5));
    }

    // Same vectors as examples/life/ref/vectors.json (Python reference).
    #[test]
    fn glider_translates_after_4() {
        let g0 = pattern("glider", 0).unwrap();
        let shifted = from_cells(&[(0, 1), (1, 2), (2, 0), (2, 1), (2, 2)], 2, 2);
        assert_eq!(run(g0, 4), shifted);
    }

    #[test]
    fn glider_python_vector() {
        let start = U256::from(0x7_0004_0002u64);
        assert_eq!(ref_step_packed(start), U256::from(0x2_0006_0005_0000u64));
        assert_eq!(run(start, 4), U256::from(0xe_0008_0004_0000u64));
    }

    #[test]
    fn blinker_period_2_and_block_still() {
        let b = pattern("blinker", 0).unwrap();
        assert_ne!(run(b, 1), b);
        assert_eq!(run(b, 2), b);
        let block = from_cells(&[(7, 7), (7, 8), (8, 7), (8, 8)], 0, 0);
        assert_eq!(run(block, 3), block);
    }

    #[test]
    fn wraps_across_corner() {
        let block = from_cells(&[(15, 15), (15, 0), (0, 15), (0, 0)], 0, 0);
        assert_eq!(run(block, 1), block);
    }

    #[test]
    fn unknown_pattern() {
        assert!(pattern("nope", 0).is_none());
    }
}
