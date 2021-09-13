#!/usr/bin/ic-repl -o
identity private "../private.pem";

let amount = (100_000_000 : nat64);  // 1e-8 ICP
let memo = (49 : nat64);

call ledger.send_dfx(
  record {
    to = neuron_account(private, memo);
    fee = record { e8s = 10_000 };
    memo = memo;
    from_subaccount = null;
    created_at_time = null;
    amount = record { e8s = amount };
  },
);

call nns.claim_or_refresh_neuron_from_account(
  record { controller = opt private; memo = memo }
);
