// Garbage collection builtin function: collectgarbage.

use num_traits::ToPrimitive;

use crate::vm::machine::VM;
use crate::vm::value::Value;

pub fn builtin_collectgarbage(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let opt = args
        .first()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "collect".to_string());
    match opt.as_str() {
        "collect" => match vm.gc_collect() {
            Ok(()) => Ok(vec![Value::Int(0)]),
            Err(err) => Err(err),
        },
        "stop" => {
            vm.gc.stop();
            Ok(vec![Value::Nil])
        }
        "restart" => {
            vm.gc.restart();
            Ok(vec![Value::Nil])
        }
        "count" => Ok(vec![Value::Float(vm.gc.count_kb())]),
        "isrunning" => Ok(vec![Value::Bool(vm.gc.is_running)]),
        "step" => {
            let _step_size = args
                .get(1)
                .and_then(|v| match v {
                    Value::Int(i) => i.to_usize(),
                    _ => None,
                })
                .unwrap_or(1024);
            // `step` historically collected a full cycle on threshold, so
            // it runs the same finalizing driver (the size is advisory and
            // was already ignored). Returns whether a collection ran.
            if vm.gc.should_collect() {
                vm.gc_collect()?;
                Ok(vec![Value::Bool(true)])
            } else {
                Ok(vec![Value::Bool(false)])
            }
        }
        _ => Err(format!("unknown collectgarbage option '{}'", opt)),
    }
}
