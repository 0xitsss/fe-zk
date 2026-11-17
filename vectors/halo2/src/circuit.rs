//! Tiny test circuit: "standard PLONK" row gate + one range lookup + copy constraints
//! + one public instance. Single phase, no challenges, all advice queried at rotation 0.
//!
//! Gate 1 (one polynomial):
//!   q_a*a + q_b*b + q_c*c + q_ab*a*b + constant = 0
//! Gate 2 (rotation): q_next*(c - a(next row)) = 0, enabled on row 0 only.
//! Lookup (one argument, one input/table expression):
//!   (q_lookup * a)  in  table      with table = {0, 1, ..., 15}
//! Copy constraints over advice a,b,c and the instance column (enable_equality).
//!
//! Witness (public out = a*b + b'):
//!   row0: a=3,  b=5,  c=15   a*b - c = 0
//!   row1: a=15 (copy of row0.c), b=7, c=22   a + b - c = 0 ; c == instance[0]
//!   row2: a=9,  b=0,  c=0    a - 9 = 0 (constant = -9)
//!   row3: a=3 (copy of row0.a), b=7 (copy of row1.b), c=21   a*b - c = 0
//! q_lookup = 1 on rows 0..=3 (a in {3,15,9,3} are all < 16).

use halo2_base::halo2_proofs::{
    circuit::{Layouter, SimpleFloorPlanner, Value},
    halo2curves::bn256::Fr,
    plonk::{
        Advice, Assigned, Circuit, Column, ConstraintSystem, Error, Fixed, Instance, TableColumn,
    },
    poly::Rotation,
};

#[derive(Clone, Copy)]
pub struct Config {
    pub a: Column<Advice>,
    pub b: Column<Advice>,
    pub c: Column<Advice>,
    pub q_a: Column<Fixed>,
    pub q_b: Column<Fixed>,
    pub q_c: Column<Fixed>,
    pub q_ab: Column<Fixed>,
    pub constant: Column<Fixed>,
    pub q_lookup: Column<Fixed>,
    pub q_next: Column<Fixed>,
    pub table: TableColumn,
    pub instance: Column<Instance>,
}

/// `x`, `y` private multiplicands; output = x*y + 7 ... see module docs for fixed rows.
#[derive(Clone, Default)]
pub struct Demo {
    pub a: Fr,
    pub b: Fr,
}

impl Demo {
    pub fn new(a: u64, b: u64) -> Self {
        Self { a: Fr::from(a), b: Fr::from(b) }
    }
    /// public output: a*b + 7
    pub fn instances(&self) -> Vec<Vec<Fr>> {
        vec![vec![self.a * self.b + Fr::from(7u64)]]
    }
    pub fn num_instance() -> Vec<usize> {
        vec![1]
    }
}

impl Circuit<Fr> for Demo {
    type Config = Config;
    type FloorPlanner = SimpleFloorPlanner;
    type Params = ();

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<Fr>) -> Config {
        let [a, b, c] = [(); 3].map(|_| meta.advice_column());
        let [q_a, q_b, q_c, q_ab, constant, q_lookup, q_next] = [(); 7].map(|_| meta.fixed_column());
        let table = meta.lookup_table_column();
        let instance = meta.instance_column();
        for col in [a, b, c] {
            meta.enable_equality(col);
        }
        meta.enable_equality(instance);

        meta.create_gate("q_a*a + q_b*b + q_c*c + q_ab*a*b + constant = 0", |meta| {
            let [a, b, c] = [a, b, c].map(|col| meta.query_advice(col, Rotation::cur()));
            let [q_a, q_b, q_c, q_ab, constant] = [q_a, q_b, q_c, q_ab, constant]
                .map(|col| meta.query_fixed(col, Rotation::cur()));
            Some(q_a * a.clone() + q_b * b.clone() + q_c * c + q_ab * a * b + constant)
        });

        // second gate: rotation query (exercises a non-zero advice rotation)
        meta.create_gate("q_next*(c - a_next) = 0", |meta| {
            let q = meta.query_fixed(q_next, Rotation::cur());
            let c = meta.query_advice(c, Rotation::cur());
            let a_next = meta.query_advice(a, Rotation::next());
            Some(q * (c - a_next))
        });

        meta.lookup("a in 0..16 when q_lookup", |meta| {
            let q = meta.query_fixed(q_lookup, Rotation::cur());
            let a = meta.query_advice(a, Rotation::cur());
            vec![(q * a, table)]
        });

        Config { a, b, c, q_a, q_b, q_c, q_ab, constant, q_lookup, q_next, table, instance }
    }

    fn synthesize(&self, cfg: Config, mut layouter: impl Layouter<Fr>) -> Result<(), Error> {
        layouter.assign_table(
            || "range16",
            |mut t| {
                for i in 0..16u64 {
                    t.assign_cell(|| "v", cfg.table, i as usize, || Value::known(Fr::from(i)))?;
                }
                Ok(())
            },
        )?;

        let one = Fr::one();
        let seven = Fr::from(7u64);
        let (a, b) = (self.a, self.b);
        let c0 = a * b;
        let c1 = c0 + seven;
        let c3 = a * seven; // row3: a * 7 (b copied from row1.b = 7)

        let mut out_cell = None;
        layouter.assign_region(
            || "main",
            |mut region| {
                let v = |x: Fr| Value::known(Assigned::Trivial(x));
                for r in 0..4 {
                    region.assign_fixed(cfg.q_lookup, r, Assigned::Trivial(one));
                }
                region.assign_fixed(cfg.q_next, 0, Assigned::Trivial(one)); // c0 == a1
                // row 0: a*b - c = 0
                let a0 = region.assign_advice(cfg.a, 0, v(a));
                region.assign_advice(cfg.b, 0, v(b));
                let c0c = region.assign_advice(cfg.c, 0, v(c0));
                region.assign_fixed(cfg.q_ab, 0, Assigned::Trivial(one));
                region.assign_fixed(cfg.q_c, 0, Assigned::Trivial(-one));
                // row 1: a + b - c = 0 with a = row0.c
                c0c.copy_advice(&mut region, cfg.a, 1);
                let b1 = region.assign_advice(cfg.b, 1, v(seven));
                let c1c = region.assign_advice(cfg.c, 1, v(c1));
                region.assign_fixed(cfg.q_a, 1, Assigned::Trivial(one));
                region.assign_fixed(cfg.q_b, 1, Assigned::Trivial(one));
                region.assign_fixed(cfg.q_c, 1, Assigned::Trivial(-one));
                out_cell = Some(c1c.cell());
                // row 2: a - 9 = 0
                region.assign_advice(cfg.a, 2, v(Fr::from(9u64)));
                region.assign_advice(cfg.b, 2, v(Fr::zero()));
                region.assign_advice(cfg.c, 2, v(Fr::zero()));
                region.assign_fixed(cfg.q_a, 2, Assigned::Trivial(one));
                region.assign_fixed(cfg.constant, 2, Assigned::Trivial(-Fr::from(9u64)));
                // row 3: a*b - c = 0 with a = row0.a, b = row1.b
                a0.copy_advice(&mut region, cfg.a, 3);
                b1.copy_advice(&mut region, cfg.b, 3);
                region.assign_advice(cfg.c, 3, v(c3));
                region.assign_fixed(cfg.q_ab, 3, Assigned::Trivial(one));
                region.assign_fixed(cfg.q_c, 3, Assigned::Trivial(-one));
                Ok(())
            },
        )?;
        layouter.constrain_instance(out_cell.unwrap(), cfg.instance, 0);
        Ok(())
    }
}
