//! Symmetric positive definite systems in 3 × 3 blocks, the shape of the
//! normal equations of a GNSS network: a block row per mark, and a block
//! wherever two marks share an observation.
//!
//! The matrix is factorised as N = L D Lᵀ, with L unit lower triangular and D
//! block diagonal, eliminating marks in minimum-degree order so that fill-in
//! stays small. Every operation is plain arithmetic on fixed-size blocks in a
//! fixed order, so results are bit-identical on every platform (D-024).
//! General-purpose linear algebra libraries choose SIMD and FMA kernels by CPU
//! at run time, which this avoids.
//!
//! [`Factor::selected_inverse`] gives the blocks of N⁻¹ on the factor's
//! pattern by the Takahashi recurrence (Takahashi, Fagan and Chen, 1973;
//! Erisman and Tinney, 1975). The pattern includes each mark's own block and
//! every pair of marks that share an observation, which is everything the
//! adjustment's statistics need, at about the cost of the factorisation.

use std::collections::{BTreeMap, BTreeSet};

use nalgebra::{Matrix3, Vector3};

/// A 3 × 3 block.
pub type Block = Matrix3<f64>;

/// A pivot this much smaller than the block's original diagonal, relative to
/// it, means the system is singular or nearly so: rounding error alone leaves
/// pivots of about 1e-16 where the matrix has a rank defect.
const PIVOT_RATIO: f64 = 1e-10;

/// A symmetric matrix of 3 × 3 blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockMatrix {
    diagonal: Vec<Block>,
    /// `off[i][j]` is block (i, j); `off[j][i]` holds its transpose.
    off: Vec<BTreeMap<usize, Block>>,
}

/// The factorisation found a pivot block that isn't safely positive definite:
/// the unknowns of block row `index` aren't determined by the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Singular {
    /// The block row.
    pub index: usize,
}

impl BlockMatrix {
    /// An `n` × `n` block matrix of zeros.
    pub fn new(n: usize) -> Self {
        BlockMatrix {
            diagonal: vec![Block::zeros(); n],
            off: vec![BTreeMap::new(); n],
        }
    }

    /// The number of block rows.
    pub fn size(&self) -> usize {
        self.diagonal.len()
    }

    /// Adds `b` to diagonal block `i`.
    pub fn add_diagonal(&mut self, i: usize, b: &Block) {
        self.diagonal[i] += b;
    }

    /// Adds `b` to block (i, j) and its transpose to block (j, i), for i ≠ j.
    pub fn add_off_diagonal(&mut self, i: usize, j: usize, b: &Block) {
        debug_assert_ne!(i, j);
        *self.off[i].entry(j).or_insert_with(Block::zeros) += b;
        *self.off[j].entry(i).or_insert_with(Block::zeros) += b.transpose();
    }

    /// Block (i, j), or `None` where it's structurally zero.
    pub fn block(&self, i: usize, j: usize) -> Option<Block> {
        if i == j {
            Some(self.diagonal[i])
        } else {
            self.off[i].get(&j).copied()
        }
    }

    /// The block rows connected to row `i` by a nonzero block.
    pub fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.off[i].keys().copied()
    }

    /// Factorises the matrix as L D Lᵀ in minimum-degree order. Ties go to the
    /// lowest block row, so the order, and every result, is reproducible.
    pub fn factorise(&self) -> Result<Factor, Singular> {
        let n = self.size();
        let mut diagonal = self.diagonal.clone();
        let mut active = self.off.clone();
        let mut queue: BTreeSet<(usize, usize)> = (0..n).map(|i| (active[i].len(), i)).collect();
        let mut factor = Factor {
            order: Vec::with_capacity(n),
            d_inverse: vec![Block::zeros(); n],
            l: vec![Vec::new(); n],
        };

        while let Some((_, k)) = queue.pop_first() {
            let d = diagonal[k];
            let d_inverse = spd_inverse(&d)
                .filter(|_| {
                    (0..3).all(|c| d[(c, c)] > PIVOT_RATIO * self.diagonal[k][(c, c)].abs())
                })
                .ok_or(Singular { index: k })?;
            let row = std::mem::take(&mut active[k]);
            // L_ik = A_ik D_k⁻¹, where A_ik is the transpose of the stored A_ki.
            let column: Vec<(usize, Block)> = row
                .iter()
                .map(|(&i, a_ki)| (i, a_ki.transpose() * d_inverse))
                .collect();

            for &i in row.keys() {
                queue.remove(&(active[i].len(), i));
                active[i].remove(&k);
            }
            // The Schur complement: A_ij -= L_ik A_kj, which fills in between
            // every pair of k's neighbours.
            for (a, (i, l_ik)) in column.iter().enumerate() {
                for (j, _) in &column[a..] {
                    let update = l_ik * row[j];
                    if i == j {
                        diagonal[*i] -= (update + update.transpose()) * 0.5;
                    } else {
                        *active[*i].entry(*j).or_insert_with(Block::zeros) -= update;
                        *active[*j].entry(*i).or_insert_with(Block::zeros) -= update.transpose();
                    }
                }
            }
            for &i in row.keys() {
                queue.insert((active[i].len(), i));
            }

            factor.order.push(k);
            factor.d_inverse[k] = d_inverse;
            factor.l[k] = column;
        }
        Ok(factor)
    }
}

/// The inverse of a symmetric positive definite 3 × 3 block, by Cholesky, or
/// `None` if it isn't positive definite.
pub fn spd_inverse(b: &Block) -> Option<Block> {
    let c = b.cholesky()?;
    let inverse = c.inverse();
    Some((inverse + inverse.transpose()) * 0.5)
}

/// The L D Lᵀ factorisation of a [`BlockMatrix`].
#[derive(Debug, Clone, PartialEq)]
pub struct Factor {
    /// Block rows in elimination order.
    order: Vec<usize>,
    /// D⁻¹, by block row.
    d_inverse: Vec<Block>,
    /// Column of L below each block row's pivot: (row eliminated later, L block),
    /// by block row, sorted by row.
    l: Vec<Vec<(usize, Block)>>,
}

impl Factor {
    /// The number of block rows.
    pub fn size(&self) -> usize {
        self.d_inverse.len()
    }

    /// The elimination order.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// The number of off-diagonal blocks in L, original and fill-in.
    pub fn off_diagonal_blocks(&self) -> usize {
        self.l.iter().map(Vec::len).sum()
    }

    /// Solves N x = b.
    pub fn solve(&self, b: &[Vector3<f64>]) -> Vec<Vector3<f64>> {
        let mut y = b.to_vec();
        for &k in &self.order {
            let y_k = y[k];
            for (i, l_ik) in &self.l[k] {
                y[*i] -= l_ik * y_k;
            }
        }
        for (y_k, d_inverse) in y.iter_mut().zip(&self.d_inverse) {
            *y_k = d_inverse * *y_k;
        }
        for &k in self.order.iter().rev() {
            let mut x_k = y[k];
            for (i, l_ik) in &self.l[k] {
                x_k -= l_ik.transpose() * y[*i];
            }
            y[k] = x_k;
        }
        y
    }

    /// The blocks of N⁻¹ on the factor's pattern, by the Takahashi recurrence:
    /// working back through the elimination order,
    /// Z_ik = −Σ_j Z_ij L_jk and Z_kk = D_k⁻¹ − Σ_i L_ikᵀ Z_ik, over the
    /// rows i, j in column k of L. Those rows were all neighbours of k when it
    /// was eliminated, so every Z_ij needed is already on the pattern.
    pub fn selected_inverse(&self) -> SelectedInverse {
        let n = self.d_inverse.len();
        let mut z = SelectedInverse {
            diagonal: vec![Block::zeros(); n],
            off: vec![BTreeMap::new(); n],
        };
        for &k in self.order.iter().rev() {
            let column = &self.l[k];
            let mut z_column = Vec::with_capacity(column.len());
            for (i, _) in column {
                let mut z_ik = Block::zeros();
                for (j, l_jk) in column {
                    z_ik -= z.pattern_block(*i, *j) * l_jk;
                }
                z_column.push((*i, z_ik));
            }
            let mut z_kk = self.d_inverse[k];
            for ((_, l_ik), (_, z_ik)) in column.iter().zip(&z_column) {
                z_kk -= l_ik.transpose() * z_ik;
            }
            z.diagonal[k] = (z_kk + z_kk.transpose()) * 0.5;
            for (i, z_ik) in z_column {
                z.off[k].insert(i, z_ik.transpose());
                z.off[i].insert(k, z_ik);
            }
        }
        z
    }
}

/// Blocks of the inverse of a [`BlockMatrix`] on its factor's pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedInverse {
    diagonal: Vec<Block>,
    off: Vec<BTreeMap<usize, Block>>,
}

impl SelectedInverse {
    /// Block (i, j) of the inverse, or `None` if it isn't on the pattern.
    pub fn block(&self, i: usize, j: usize) -> Option<Block> {
        if i == j {
            Some(self.diagonal[i])
        } else {
            self.off[i].get(&j).copied()
        }
    }

    /// Block (i, j) during the recurrence, where it's always on the pattern.
    fn pattern_block(&self, i: usize, j: usize) -> Block {
        // The pattern is closed under elimination, so this is never zero by
        // omission; the tests check against a dense inverse.
        self.block(i, j).unwrap_or_else(Block::zeros)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::Rng;
    use nalgebra::DMatrix;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A random symmetric positive definite 3 × 3 block.
    fn spd(rng: &mut Rng) -> Block {
        let a = Block::from_fn(|_, _| rng.uniform(-1.0, 1.0));
        a * a.transpose() + Block::identity() * 0.5
    }

    /// Normal equations of a random network: `marks` marks, `links` random
    /// pairs with a random weight block each, and one weighted mark (control).
    fn network(rng: &mut Rng, marks: usize, links: usize) -> BlockMatrix {
        let mut n = BlockMatrix::new(marks);
        for k in 0..links {
            // A chain keeps the network connected; the rest are random.
            let (i, j) = if k + 1 < marks {
                (k, k + 1)
            } else {
                let i = rng.below(marks);
                (i, (i + 1 + rng.below(marks - 1)) % marks)
            };
            let p = spd(rng);
            n.add_diagonal(i, &p);
            n.add_diagonal(j, &p);
            n.add_off_diagonal(i, j, &(-p));
        }
        n.add_diagonal(rng.below(marks), &spd(rng));
        n
    }

    fn dense(n: &BlockMatrix) -> DMatrix<f64> {
        let size = n.size();
        DMatrix::from_fn(3 * size, 3 * size, |r, c| {
            n.block(r / 3, c / 3).map_or(0.0, |b| b[(r % 3, c % 3)])
        })
    }

    #[test]
    fn solves_and_inverts_like_dense() {
        let mut rng = Rng::new(11);
        for (marks, links) in [(1, 0), (2, 1), (7, 9), (30, 60), (60, 200)] {
            let n = network(&mut rng, marks, links);
            let f = n.factorise().unwrap();
            let b: Vec<Vector3<f64>> = (0..marks)
                .map(|_| Vector3::from_fn(|_, _| rng.uniform(-1.0, 1.0)))
                .collect();
            let x = f.solve(&b);
            let full = dense(&n);
            let inverse = full.clone().cholesky().unwrap().inverse();
            let b_dense = DMatrix::from_fn(3 * marks, 1, |r, _| b[r / 3][r % 3]);
            let x_dense = &inverse * b_dense;
            for (k, x_k) in x.iter().enumerate() {
                for c in 0..3 {
                    assert!((x_k[c] - x_dense[3 * k + c]).abs() < 1e-9, "{marks} marks");
                }
            }
            // Every block on the pattern matches the dense inverse.
            let z = f.selected_inverse();
            let mut checked = 0;
            for i in 0..marks {
                for j in 0..marks {
                    if let Some(block) = z.block(i, j) {
                        let d = inverse.view((3 * i, 3 * j), (3, 3));
                        assert!((block - d).abs().max() < 1e-9, "({i}, {j})");
                        checked += 1;
                    }
                }
            }
            // At least every block of N itself.
            for i in 0..marks {
                for j in n.neighbours(i) {
                    assert!(z.block(i, j).is_some());
                }
            }
            assert!(checked >= marks);
        }
    }

    #[test]
    fn a_rank_defect_is_reported() {
        // Differences only: the network can translate freely.
        let mut rng = Rng::new(5);
        let mut n = BlockMatrix::new(4);
        for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            let p = spd(&mut rng);
            n.add_diagonal(i, &p);
            n.add_diagonal(j, &p);
            n.add_off_diagonal(i, j, &(-p));
        }
        assert!(n.factorise().is_err());
        // Fixing it with a weight on one mark makes it solvable.
        n.add_diagonal(2, &Block::identity());
        assert!(n.factorise().is_ok());
    }

    #[test]
    fn minimum_degree_keeps_a_chain_free_of_fill() {
        // A long chain needs no fill in a good order.
        let mut n = BlockMatrix::new(200);
        for i in 0..199 {
            n.add_diagonal(i, &Block::identity());
            n.add_diagonal(i + 1, &Block::identity());
            n.add_off_diagonal(i, i + 1, &(-Block::identity()));
        }
        n.add_diagonal(100, &Block::identity());
        let f = n.factorise().unwrap();
        assert_eq!(f.off_diagonal_blocks(), 199);
        assert_eq!(f.order().len(), 200);
    }
}
