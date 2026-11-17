pragma circom 2.0.0;

// c = a * b; c is the only public signal (1 public input).
template Multiplier() {
    signal input a;
    signal input b;
    signal output c;
    c <== a * b;
}

component main = Multiplier();
