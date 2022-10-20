function f(x) {
  let _ = x.id;
};
function f2(x) { let _ = record { abc = x.id; } };
function f3(x) { let _ = x.y };
function f4(acc, x) { let _ = stringify(acc, x, ",") };
let x = vec{record {id=1;x=opt 2};record {id=2;y=opt 5}};
assert x.map(f) == vec {1;2};
assert x.map(f2) == vec {record { abc = 1 }; record { abc = 2 }};
assert x.filter(f3) == vec { record {id=2; y=opt 5}};
assert x.filter(f3).map(f) == vec {2};
assert x.map(f).fold("", f4) == "1,2,";

let y = vec { variant { y = 1 }; variant { x = "error" }; variant { y = 2 } };
assert y.filter(f3).map(f3) == vec {1;2};

let z = record { opt 1;2;opt 3;opt 4 };
function f5(x) { let _ = record { x[0]; x[1]? } };
function f6(x) { let _ = x[1]? };
assert z.filter(f6).map(f5) == record { 1; 2 = 3; 4 }; 

