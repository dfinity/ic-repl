# Canister REPL

```
ic-repl --replica [local|ic|url] --config <dhall config> [script file]
```

## Commands

```
<command> := 
 | import <id> = <text> [ : <text> ]   (canister URI with optional did file)
 | export <text>  (filename)
 | load <text>    (filename)
 | config <text>  (dhall config)
 | call <name> . <name> ( <val>,* )
 | let <id> = <val>
 | show <val>
 | assert <val> <binop> <val>
 | identity <id>

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
