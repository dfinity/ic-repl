# Canister REPL

```
ic-repl --replica [local|ic|url] --config <dhall config> [script file]
```

## Commands

```
<command> := 
 | import <id> = <text>   (canister URI)
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
