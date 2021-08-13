#!/ic-repl
identity alice;
let id = call ic.provisional_create_canister_with_cycles(record { settings = null; amount = null });
call ic.canister_status(id);
assert _.module_hash == (null : opt blob);
call ic.install_code(
  record {
    arg = encode ();
    wasm_module = file "greet.wasm";
    mode = variant { install };
    canister_id = id.canister_id;
  },
);
let status = call ic.canister_status(id);
assert status.settings ~= record { controllers = vec { alice } };
assert status.module_hash? == blob "\d8\d1\d3;\a3\a65\a6\a6\c8!\06\12\d2\da\9dZ\e4v\8d\27\bd\05\9d\cc\1a\df\cb \01u\dc";
let canister = id.canister_id;
call canister.greet("test");
assert _ == "Hello, test!";
call ic.stop_canister(id);
call ic.delete_canister(id);
