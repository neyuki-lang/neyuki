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
        "collect" => {
            vm.gc.collect_garbage(&vm.stack, &vm.globals);
            Ok(vec![Value::Int(0)])
        }
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
            let step_size = args
                .get(1)
                .and_then(|v| match v {
                    Value::Int(i) => i.to_usize(),
                    _ => None,
                })
                .unwrap_or(1024);
            let collected = vm.gc.step(step_size, &vm.stack, &vm.globals);
            Ok(vec![Value::Bool(collected)])
        }
        _ => Err(format!("unknown collectgarbage option '{}'", opt)),
    }
}
