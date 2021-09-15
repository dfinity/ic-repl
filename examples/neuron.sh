#!/usr/bin/ic-repl -o
identity private "../private.pem";

// Staking or top up

let amount = 100_000_000;  // 1 ICP
let memo = 42;  // memo determines neuron id

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

let neuron_id = 3543344363;  // The neuron_id is the return of the previous method call

// Define neuron config operations
let dissolve_delay = variant {
  IncreaseDissolveDelay = record {
    additional_dissolve_delay_seconds = 3_600;
  }
};
let start_dissolving = variant {
  StartDissolving = record {}
};
let stop_dissolving = variant {
  StopDissolving = record {}
};
let hot_key = principal "aaaaa-aa";
let add_hot_key = variant {
  AddHotKey = record { new_hot_key = opt hot_key }
};
let remove_hot_key = variant {
  RemoveHotKey = record { hot_key_to_remove = opt hot_key }
};

// Choose a specific operation above to execute
call nns.manage_neuron(
  record {
    id = opt record { id = neuron_id };
    command = opt variant {
      Configure = record {
        operation = opt dissolve_delay;
      }
    };
    neuron_id_or_subaccount = null;
  },
);

// Disburse
call nns.manage_neuron(
  record {
    id = opt record { id = neuron_id };
    command = opt variant {
      Disburse = record { to_account = null; amount = null }
    };
    neuron_id_or_subaccount = null;
  },
);
