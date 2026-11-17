# Reference values for src/tests/ec.fe, computed with py_ecc (independent of the Fe code).
# Run: python3 -I scripts/ec-vectors.py   (needs `pip install py_ecc`; tested with py_ecc 8.0.0)
from py_ecc.bn128 import (G1, G2, add, multiply, neg, curve_order, field_modulus as q,
                          b2, is_on_curve, is_inf, FQ2)

def g1(p): return (int(p[0]), int(p[1]))
def g2(p): return tuple(int(c) for c in (p[0].coeffs[0], p[0].coeffs[1], p[1].coeffs[0], p[1].coeffs[1]))
def pr(name, v): print(f"{name} = {v}")

# --- G1 ---
s = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef % curve_order
two = multiply(G1, 2)
pr("G1 2G  (multiply(G1,2))", g1(two))
pr("G1 3G  (add(G1,multiply(G1,2)))", g1(add(G1, two)))
pr("G1 s*G (multiply(G1,s)), s=%d" % s, g1(multiply(G1, s)))
pr("G1 s*2G", g1(multiply(two, s)))
pr("G1 (r-1)*G == -G", g1(multiply(G1, curve_order - 1)))
pr("G1 -G (neg(G1))", g1(neg(G1)))
assert multiply(G1, curve_order) is None

# --- G2 ---
pr("G2 gen (x0,x1,y0,y1)", g2(G2))
pr("G2 2*G2", g2(multiply(G2, 2)))
a = 123456789
pr("G2 a*G2, a=%d" % a, g2(multiply(G2, a)))
pr("G1 a*G1", g1(multiply(G1, a)))

def fq2_pow(x, e):
    r = FQ2([1, 0])
    while e:
        if e & 1: r = r * x
        x = x * x; e >>= 1
    return r
def fq2_sqrt(a):
    # q = 3 mod 4: Adj & Rodriguez-Henriquez Alg. 9
    a1 = fq2_pow(a, (q - 3) // 4)
    alpha = a1 * a1 * a
    x0 = a1 * a
    if alpha == FQ2([-1, 0]):
        return x0 * FQ2([0, 1])
    b = fq2_pow(alpha + FQ2([1, 0]), (q - 1) // 2)
    return b * x0
P = None
for k in range(1, 100):
    x = FQ2([k, 1])
    rhs = x ** 3 + b2
    y = fq2_sqrt(rhs)
    if y * y == rhs:
        cand = (x, y)
        assert is_on_curve(cand, b2)
        if multiply(cand, curve_order) is not None:
            P = cand
            pr("G2 non-subgroup on-curve point (x0,x1,y0,y1)", g2(P))
            break

xi = FQ2([9, 1])
g1c = fq2_pow(xi, (q - 1) // 3); g2c = fq2_pow(xi, (q - 1) // 2)
pr("gamma1 (c0,c1)", (int(g1c.coeffs[0]), int(g1c.coeffs[1])))
pr("gamma2 (c0,c1)", (int(g2c.coeffs[0]), int(g2c.coeffs[1])))
pr("b2 = 3/(9+u) (c0,c1)", (int(b2.coeffs[0]), int(b2.coeffs[1])))
def psi(Pt):
    x, y = Pt
    cx = FQ2([x.coeffs[0], -x.coeffs[1]]); cy = FQ2([y.coeffs[0], -y.coeffs[1]])
    return (cx * g1c, cy * g2c)
X = 4965661367192848881
pr("6x^2", 6 * X * X)
assert psi(G2) == multiply(G2, 6 * X * X)
assert psi(P) != multiply(P, 6 * X * X)
