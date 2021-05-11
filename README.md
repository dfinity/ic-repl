# Canister REPL

```
ic-repl --replica [local|ic|url] --config <dhall config> [script file]
```

## Commands

```
<command> := 
 | import <id> = <text> (as <text>)?     // bind canister URI to <id>, with optional did file
 | export <text>                         // export command history to a file that can be run in ic-repl as a script
 | load <text>                           // load and run a script file
 | config <text>                         // set config for random value generator in dhall format
 | let <id> = <exp>                      // bind <exp> to a variable <id>
 | <exp>                                 // show the value of <exp>
 | assert <exp> <binop> <exp>            // assertion
 | identity <id> <text>?                 // switch to identity <id>, with optional Ed25519 pem file
<exp> := 
 | <candid val>                          // any candid value
 | <var> <selector>*                     // variable with optional selectors
 | file <text>                           // load external file as a blob value
 | call <name> . <name> ( <exp>,* )      // call a canister method, and store the result as a single value
 | encode (<name> . <name>)? ( <exp>,* ) // encode candid arguments as a blob value
 | decode (as <name> . <name>)? <exp>    // decode blob as candid values
<var> := 
 | <id>                  // variable name 
 | _                     // previous eval of exp is bind to `_` 
<selector> :=
 | ?                     // select opt value
 | . <name>              // select field name from record or variant value
 | [ <nat> ]             // select index from vec, record, or variant value
<binop> := 
 | ==                    // structural equality
 | ~=                    // equal under candid subtyping
 | !=                    // not equal
```

## Example

test.sh
```
#!/usr/bin/ic-repl -r ic

import greet = "rrkah-fqaaa-aaaaa-aaaaq-cai";
call greet.greet("test");
let result = _;
assert _ == "Hello, test!";
identity alice;
call "rrkah-fqaaa-aaaaa-aaaaq-cai".greet("test");
assert _ == result;
```

install.sh
```
#!/usr/bin/ic-repl
let id = call "aaaaa-aa".provisional_create_canister_with_cycles(record { settings: null; amount: null });
call "aaaaa-aa".install_code(
  record {
    arg = encode ();
    wasm_module = file "your_wasm_file.wasm";
    mode = variant { install };
    canister_id = id.canister_id;
  },
);
call "aaaaa-aa".canister_status(id);
let canister = id.canister_id;
call canister.greet("test");
```

wallet.sh
```
#!/usr/bin/ic-repl
import wallet = "rwlgt-iiaaa-aaaaa-aaaaa-cai" as "wallet.did";
identity default "~/.config/dfx/identity/default/identity.pem";
call wallet.wallet_create_canister(
  record {
    cycles = 824_567_85;
    settings = record {
      controller = null;
      freezing_threshold = null;
      memory_allocation = null;
      compute_allocation = null;
    };
  },
)
let id = _.Ok.canister_id;
let msg = encode "aaaaa-aa".install_code(
  record {
    arg = encode ();
    wasm_module = file "your_wasm_file.wasm";
    mode = variant { install };
    canister_id = id;
  },
);
let res = call wallet.wallet_call(
  record {
    args = msg;
    cycles = 0;
    method_name = "install_code";
    canister = principal "aaaaa-aa";
  },
);
decode as "aaaaa-aa".install_code res.Ok.return;
call id.greet("test");
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

If you are writing your own `.did` file, you can also supply the did file via the `import` command, e.g. `import canister = "rrkah-fqaaa-aaaaa-aaaaq-cai" : "your_own_did_file.did"`

## Issues

* Acess to service init type
* `IDLValue::Blob` for efficient blob serialization
* Tokenization for partial parser (variable needs a preceding space for autocompletion)
* Rename `Value` module to `Exp`
* Autocompletion within Candid value; autocompletion for decode method
* Robust support for `~=`, requires inferring principal types
* Loop detection for `load`
* Assert upgrade correctness
