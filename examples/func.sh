function f(x) {
  let _ = x.id;
};
function f2(x) { let _ = record { abc = x.id; } };
function f3(x) { let _ = exist(x.y) };
function f3_2(x) { let _ = x.y };
function f4(acc, x) { let _ = add(acc, x) };
let x = vec{record {id=1;x=opt 2};record {id=2;y=opt 5}};
assert x.map(f) == vec {1;2};
assert x.map(f2) == vec {record { abc = 1 }; record { abc = 2 }};
assert x.filter(f3) == vec { record {id=2; y=opt 5}};
assert x.filter(f3).map(f) == vec {2};
assert x.map(f).fold(0, f4) == 3;

let y = vec { variant { y = 1 }; variant { x = "error" }; variant { y = 2 } };
assert y.filter(f3).map(f3_2) == vec {1;2};
assert y[sub(y.size(), 1)].y == 2;

let z = record { opt 1;2;opt 3;opt 4 };
function f5(x) { let _ = record { x[0]; x[1]? } };
function f6(x) { let _ = exist(x[1]?) };
function f7(acc, x) { let _ = concat(acc, vec{x[1]}) };
assert z.filter(f6).map(f5) == record { 1; 2 = 3; 4 }; 
assert z.filter(f6).map(f5).fold(vec{}, f7) == vec {1;3;4};
assert z[sub(z.size(), 1)]? == 4;

let s = "abcdef";
function f8(x) { let _ = stringify(" ", x) };
function f9(acc, x) { let _ = add(acc, 1) };
assert s.map(f8) == " a b c d e f";
assert s.map(f8).fold(0, f9) == 12;
assert s.map(f8).size() == (12 : nat);
assert s[sub(s.size(), 1)] == "f";

assert div(1, 2) == 0;
assert div(1, 2.0) == 0.5;
assert div((mul(div(((1:nat8):float32), (3:float64)), 1000) : nat), 100.0) == 3.33;
assert eq("text", "text") == true;
assert not(eq("text", "text")) == false;
assert eq(div(1,2), sub(2,2)) == true;
assert gt(div(1, 2.0), 1) == false;
assert eq(div(1, 2.0), 0.5) == true;
assert and(lte(div(1, 2), 0), gte(div(1, 2), 0)) == true;
assert or(lt(div(1, 2), 0), gt(div(1, 2), 0)) == false;

assert (service "aaaaa-aa" : principal) == principal "aaaaa-aa";
assert eq((service "aaaaa-aa" : principal), principal "aaaaa-aa") == true;
assert (func "aaaaa-aa".test : service {}) == service "aaaaa-aa";
assert (principal "aaaaa-aa" : service {}) == service "aaaaa-aa";

assert ("this is a text" : blob) == blob "this is a text";
assert (blob "this is a blob" : text) == "this is a blob";

function fac(n) {
  if eq(n, 0) {
      let _ = 1;
  } else {
      let _ = mul(n, fac(sub(n, 1)));
  }
};
function fac2(n) {
  let res = 1;
  while gt(n, 0) {
      let res = mul(res, n);
      let n = sub(n, 1);
  };
  let _ = res;
};
function fac3(n) {
  let _ = ite(eq(n, 0), 1, mul(n, fac3(sub(n, 1))))
};
function fib(n) {
  let _ = ite(lt(n, 2), 1, add(fib(sub(n, 1)), fib(sub(n, 2))))
};
function fib2(n) {
  let a = 1;
  let b = 1;
  while gt(n, 0) {
      let b = add(a, b);
      let a = sub(b, a);
      let n = sub(n, 1);
  };
  let _ = a;
};
function fib3(n) {
  if lt(n, 2) {
      let _ = 1;
  } else {
      let _ = add(fib3(sub(n, 1)), fib3(sub(n, 2)));
  }
};
function __main() {
assert fac(5) == 120;
assert fac2(5) == 120;
assert fac3(5) == 120;
assert fib(10) == 89;
assert fib2(10) == 89;
assert fib3(10) == 89;
}
