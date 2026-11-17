pragma circom 2.0.0;

// c = a * b; public signals: c (output) then a (public input) = 2 public inputs.
template Multiplier2() {
    signal input a;
    signal input b;
    signal output c;
    c <== a * b;
}

component main {public [a]} = Multiplier2();
