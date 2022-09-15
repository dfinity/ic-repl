#!/usr/bin/ic-repl
function deploy(wallet, wasm, cycle) {
  identity default "~/.config/dfx/identity/default/identity.pem";
  call wallet.wallet_create_canister(
    record {
      cycles = cycle;
      settings = record {
        controller = null;
        freezing_threshold = null;
        memory_allocation = null;
        compute_allocation = null;
      };
    },
  );
  let id = _.Ok.canister_id;
  call as wallet ic.install_code(
    record {
      arg = encode ();
      wasm_module = wasm;
      mode = variant { install };
      canister_id = id;
    },
  );
  id
};

import wallet = "${WALLET_ID:-rwlgt-iiaaa-aaaaa-aaaaa-cai}" as "wallet.did";
let id = deploy(wallet, file("greet.wasm"), 1_000_000);
call id.greet("test");
