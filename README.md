# Canister REPL

```
ic-repl --replica [local|ic|url] --config <dhall config> [script file]
```

## Commands

```
<command> := 
 | import <id> = <text> [ : <text> ]    // bind canister URI to <id>, with optional did file
 | call <name> . <name> ( <val>,* )     // call a canister method with candid arguments
 | encode <name> . <name> ( <val>,* )   // encode candid arguments with respect to a canister method signature
 | export <text>                        // export command history to a file that can be run in ic-repl as a script
 | load <text>                          // load and run a script file
 | config <text>                        // set config for random value generator in dhall format
 | let <id> = <val>                     // bind <val> to a variable <id>
 | show <val>                           // show the value of <val>
 | assert <val> <binop> <val>           // assertion
 | identity <id>                        // switch to identity <id> (create a new one if doesn't exist)

<var> := <id> | _
<val> := <candid val> | <var> (. <id>)* | file <text>
<binop> := == | ~= | !=
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
#!/usr/bin/ic-repl -r ic
call "aaaaa-aa".provisional_create_canister_with_cycles(record { settings: null; amount: null });
let id = _;
call "aaaaa-aa".install_code(
  record {
    arg = blob "";
    wasm_module = file "your_wasm_file.wasm";
    mode = variant { install };
    canister_id = id.canister_id; // TODO
  },
);
call "aaaaa-aa".canister_status(id);
call id.canister_id.greet("test");
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

* Autocompletion within Candid value
* Robust support for `~=`, requires inferring principal types
* Value projection
* Bind multiple return values to `_`
* Loop detection for `load`
* Import external identity
* Assert upgrade correctness
