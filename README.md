# Canister REPL

```
ic-repl --replica [local|ic|url] --config <dhall config> [script file]
```

## Commands

```
<command> := 
 | import <id> = <text> (as <text>)?     // bind canister URI to <id>, with optional did file
 | export <text>                         // export current environment variables
 | load <text>                           // load and run a script file
 | config <text>                         // set config for random value generator in dhall format
 | let <id> = <exp>                      // bind <exp> to a variable <id>
 | <exp>                                 // show the value of <exp>
 | assert <exp> <binop> <exp>            // assertion
 | identity <id> <text>?                 // switch to identity <id>, with optional Ed25519 pem file
<exp> := 
 | <candid val>                                    // any candid value
 | <var> <selector>*                               // variable with optional selectors
 | file <text>                                     // load external file as a blob value
 | fail <exp>                                      // convert error message as text
 | call (as <name>)? <name> . <name> ( <exp>,* )   // call a canister method, and store the result as a single value
 | encode (<name> . <name>)? ( <exp>,* )           // encode candid arguments as a blob value
 | decode (as <name> . <name>)? <exp>              // decode blob as candid values
<var> := 
 | <id>                  // variable name 
 | _                     // previous eval of exp is bind to `_` 
<selector> :=
 | ?                     // select opt value
 | . <name>              // select field name from record or variant value
 | [ <nat> ]             // select index from vec, record, or variant value
<binop> := 
 | ==                    // structural equality
 | ~=                    // equal under candid subtyping; for text value, we check if the right side is contained in the left side
 | !=                    // not equal
```

## Example

### test.sh
```
#!/usr/bin/ic-repl -r ic
// assume we already installed the greet canister
import greet = "rrkah-fqaaa-aaaaa-aaaaq-cai";
call greet.greet("test");
let result = _;
assert _ == "Hello, test!";
identity alice;
call "rrkah-fqaaa-aaaaa-aaaaq-cai".greet("test");
assert _ == result;
```

### nns.sh
```
#!/usr/bin/ic-repl -r ic
// nns and ledger canisters are auto-imported if connected to the mainnet
call nns.get_pending_proposals()
call ledger.account_balance_dfx(record { account = "..." })
```

### install.sh
```
#!/usr/bin/ic-repl
identity alice;
let id = call "aaaaa-aa".provisional_create_canister_with_cycles(record { settings = null; amount = null });
call ic.install_code(
  record {
    arg = encode ();
    wasm_module = file "greet.wasm";
    mode = variant { install };
    canister_id = id.canister_id;
  },
);
let status = call ic.canister_status(id);
assert status.settings ~= record { controller = alice };
assert status.module_hash? == blob "...";
let canister = id.canister_id;
call canister.greet("test");
```

### wallet.sh
```
#!/usr/bin/ic-repl
import wallet = "${WALLET_ID:-rwlgt-iiaaa-aaaaa-aaaaa-cai}" as "wallet.did";
identity default "~/.config/dfx/identity/default/identity.pem";
call wallet.wallet_create_canister(
  record {
    cycles = ${CYCLE:-1_000_000};
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
    wasm_module = file "${WASM_FILE}";
    mode = variant { install };
    canister_id = id;
  },
);
call id.greet("test");
```

## Derived forms

* `call as proxy_canister target_canister.method(args)` is a shorthand for
```
let _ = call proxy_canister.wallet_call(
  record {
    args = encode target_canister.method(args);
    cycles = 0;
    method_name = "method";
    canister = principal "target_canister";
  }
);
decode as target_canister.method _.Ok.return
```

## Notes for Rust canisters

`ic-repl` relies on the `__get_candid_interface_tmp_hack` canister method to fetch the Candid interface. The default
Rust CDK does not provide this method. You can do the following to enable this feature:

* For each canister method, in addition to the `#[ic_cdk_macros::query]` annotation, add `#[ic_cdk::export::candid::candid_method(query)]` or `#[ic_cdk::export::candid::candid_method]` for query and update calls respectively.
* At the end of the the canister `.rs` file, add the following lines:
```
ic_cdk::export::candid::export_service!();

#[ic_cdk_macros::query(name = "__get_candid_interface_tmp_hack")]
fn export_candid() -> String {
    __export_service()
}
```

If you are writing your own `.did` file, you can also supply the did file via the `import` command, e.g. `import canister = "rrkah-fqaaa-aaaaa-aaaaq-cai" as "your_own_did_file.did"`

## Issues

* Acess to service init type
* `IDLValue::Blob` for efficient blob serialization
* Autocompletion within Candid value
* Robust support for `~=`, requires inferring principal types
* Loop detection for `load`
* Assert upgrade correctness
