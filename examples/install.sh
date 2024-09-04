#!/ic-repl
function deploy(wasm) {
  let id = call ic.provisional_create_canister_with_cycles(record { settings = null; amount = null });
  call ic.canister_status(id);
  assert _.module_hash == (null : opt blob);
  call ic.install_code(
    record {
      arg = encode wasm.__init_args();
      wasm_module = wasm;
      mode = variant { install };
      canister_id = id.canister_id;
    },
  );
  id
};

identity alice;
let id = deploy(file("greet.wasm"));
let canister = id.canister_id;
let res = par_call [ic.canister_status(id), canister.greet("test")];
let status = res[0];
assert status.settings ~= record { controllers = vec { alice } };
assert status.module_hash? == blob "\ab\a7h\8cH\e0]\e7W]\8b\07\92\ac\9fH\95\7f\f4\97\d0\efX\c4~\0d\83\91\01<\da\1d";
assert res[1] == "Hello, test!";
call ic.stop_canister(id);
call ic.delete_canister(id);
