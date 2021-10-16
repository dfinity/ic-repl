#!/usr/bin/ic-repl -o
identity private "../private.pem";

function transfer(to, amount, memo) {
  call ledger.send_dfx(
    record {
      to = to;
      fee = record { e8s = 10_000 };
      memo = memo;
      from_subaccount = null;
      created_at_time = null;
      amount = record { e8s = amount };
    },
  );
};

// Staking or top up
function stake(amount, memo) {
  let _ = transfer(neuron_account(private, memo), amount, memo);
  call nns.claim_or_refresh_neuron_from_account(
    record { controller = opt private; memo = memo }
  );
  _.result?.NeuronId
};

let amount = 100_000_000;  // 1 ICP
let memo = 42;  // memo determines neuron id
let neuron_id = stake(amount, memo);

// Define neuron config operations
function dissolve_delay(delay) {
  variant {
    IncreaseDissolveDelay = record {
      additional_dissolve_delay_seconds = delay;
    }
  }
};
function start_dissolving() {
  variant {
    StartDissolving = record {}
  }
};
function stop_dissolving() {
  variant {
    StopDissolving = record {}
  }
};
function add_hot_key(hot_key) {
  variant {
    AddHotKey = record { new_hot_key = opt hot_key }
  }
};
function remove_hot_key(hot_key) {
  variant {
    RemoveHotKey = record { hot_key_to_remove = opt hot_key }
  }
};
function config_neuron(neuron_id, operation) {
  let _ = call nns.manage_neuron(
    record {
      id = opt record { id = neuron_id };
      command = opt variant {
        Configure = record {
          operation = opt operation;
        }
      };
      neuron_id_or_subaccount = null;
    },
  );
};

config_neuron(neuron_id, dissolve_delay(3600));

function disburse() {
  variant { Disburse = record { to_account = null; amount = null } }
};
function spawn() {
  variant { Spawn = record { new_controller = null } }
};
function merge_maturity(percent) {
  variant { MergeMaturity = record { percentage_to_merge = percent } }
};
function manage(neuron_id, cmd) {
  let _ = call nns.manage_neuron(
    record {
      id = opt record { id = neuron_id };
      command = opt cmd;
      neuron_id_or_subaccount = null;
    },
  )
};

manage(neuron_id, disburse());
