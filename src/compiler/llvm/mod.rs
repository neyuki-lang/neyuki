//! Textual LLVM IR backend lowering from [`MirProgram`].

pub mod codegen;
pub mod emit;
pub mod native;
pub mod runtime;
pub mod target;
pub mod types;

pub use emit::{LlvmOptions, emit_llvm_ir, emit_llvm_ir_with};
pub use runtime::RuntimeMode;
pub use target::TargetSpec;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::hir::lower_ast;
    use crate::compiler::mir::{lower_thir_to_mir, optimize_mir};
    use crate::compiler::thir::lower_hir_to_thir;
    use crate::compiler::{OptLevel, compile_source};

    #[test]
    fn test_emit_llvm_ir_contains_entry_runtime_and_functions() {
        let code = "\
local function fib(n: int): int
  if n <= 1 then
    return n
  end
  return fib(n - 1) + fib(n - 2)
end
print(fib(10))";
        let stmts = compile_source(code).expect("syntax error");
        let hir = lower_ast(&stmts, None, OptLevel::O1);
        let thir = lower_hir_to_thir(&hir);
        let mut mir = lower_thir_to_mir(&thir, false);
        optimize_mir(&mut mir, OptLevel::O1);

        let ll = emit_llvm_ir(&mir, OptLevel::O1);
        assert!(ll.contains("%NykVal = type { i8, i64 }"));
        assert!(ll.contains("define %NykVal @nyk_fn_0_0_fib("));
        assert!(ll.contains("define %NykVal @nyk_main()"));
        assert!(ll.contains("define i32 @main()"));
        assert!(ll.contains("call void @__nyk_print_one("));
    }

    #[test]
    fn test_emit_llvm_ir_numeric_for_and_strings() {
        let code = "\
local sum = 0
for i = 1, 5 do
  sum = sum + i
end
print(\"sum\", sum)";
        let stmts = compile_source(code).expect("syntax error");
        let hir = lower_ast(&stmts, None, OptLevel::O1);
        let thir = lower_hir_to_thir(&hir);
        let mut mir = lower_thir_to_mir(&thir, false);
        optimize_mir(&mut mir, OptLevel::O1);

        let ll = emit_llvm_ir(&mir, OptLevel::O1);
        assert!(ll.contains("@.str.0 = private unnamed_addr constant [4 x i8] c\"sum\\00\""));
        assert!(ll.contains("call %NykVal @__nyk_add("));
        assert!(ll.contains("call %NykVal @__nyk_le("));
    }

    fn emit(code: &str) -> String {
        emit_with(code, &LlvmOptions::default())
    }

    fn emit_with(code: &str, options: &LlvmOptions) -> String {
        let stmts = compile_source(code).expect("syntax error");
        let hir = lower_ast(&stmts, None, OptLevel::O1);
        let thir = lower_hir_to_thir(&hir);
        let mut mir = lower_thir_to_mir(&thir, false);
        optimize_mir(&mut mir, OptLevel::O1);
        emit_llvm_ir_with(&mir, OptLevel::O1, options)
    }

    /// A program that calls every runtime function.
    const USES_THE_RUNTIME: &str = "\
local t = { 1, 2 }
t.x = #\"ab\" .. \"c\"
local y = t.x
local v = Vector3.new(8, 1, 8)
local n = 0
local function bump() n = n + 1 return n end
local function va(...) return ... end
print(y, v, bump(), va(1))";

    /// `(name, parameter types)` of every `declare` of a runtime function.
    fn runtime_declares(ll: &str) -> Vec<(String, String)> {
        ll.lines()
            .filter_map(|line| line.strip_prefix("declare void @__nyk_"))
            .map(|rest| {
                let (name, params) = rest.split_once('(').expect("declare has params");
                let params = params.split(')').next().unwrap_or("");
                (name.to_string(), params.to_string())
            })
            .collect()
    }

    #[test]
    fn test_extern_runtime_only_declares_runtime_functions() {
        let options = LlvmOptions {
            runtime: RuntimeMode::Extern,
            target: None,
        };
        let ll = emit_with(USES_THE_RUNTIME, &options);
        let declared: Vec<String> = runtime_declares(&ll).into_iter().map(|(n, _)| n).collect();
        for name in [
            "error",
            "global_get",
            "global_set",
            "upval_get",
            "upval_set",
            "import_get",
            "table_new",
            "table_get",
            "table_set",
            "table_append",
            "concat",
            "len",
            "closure",
            "call_dyn",
            "vararg",
        ] {
            assert!(declared.iter().any(|d| d == name), "{} not declared", name);
            assert!(
                !ll.contains(&format!("define internal void @__nyk_{}(", name)),
                "{} is defined in extern mode",
                name
            );
        }
        assert_eq!(declared.len(), 15, "{:?}", declared);
        // The small helpers stay defined, so they still inline.
        for helper in ["mk_int", "truthy", "as_f64", "add", "idiv", "eq", "lt"] {
            assert!(
                ll.contains(&format!("@__nyk_{}(", helper)) && ll.contains("alwaysinline"),
                "{} missing",
                helper
            );
            assert!(
                ll.contains(&format!("define internal %NykVal @__nyk_{}(", helper))
                    || ll.contains(&format!("define internal i1 @__nyk_{}(", helper))
                    || ll.contains(&format!("define internal double @__nyk_{}(", helper))
            );
        }
    }

    #[test]
    fn test_builtin_runtime_matches_the_extern_signatures() {
        let builtin = emit(USES_THE_RUNTIME);
        let extern_ll = emit_with(
            USES_THE_RUNTIME,
            &LlvmOptions {
                runtime: RuntimeMode::Extern,
                target: None,
            },
        );
        for (name, params) in runtime_declares(&extern_ll) {
            let def_start = format!("define internal void @__nyk_{}(", name);
            let line = builtin
                .lines()
                .find(|l| l.starts_with(&def_start))
                .unwrap_or_else(|| panic!("{} not defined in builtin mode", name));
            let def_params = line[def_start.len()..].split(')').next().unwrap_or("");
            // Drop the parameter names: `i8 zeroext %tag` -> `i8 zeroext`.
            let types: Vec<String> = def_params
                .split(", ")
                .filter(|p| !p.is_empty())
                .map(|p| {
                    p.split(' ')
                        .filter(|w| !w.starts_with('%'))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            assert_eq!(types.join(", "), params, "signature of {} differs", name);
        }
    }

    #[test]
    fn test_runtime_calls_pass_no_aggregates() {
        // At the runtime boundary values are (i8, i64) pairs and results come
        // back through an out-pointer (see runtime.rs), never `%NykVal` by
        // value, which the Windows x64 C ABI passes differently.
        for mode in [RuntimeMode::Builtin, RuntimeMode::Extern] {
            let ll = emit_with(
                USES_THE_RUNTIME,
                &LlvmOptions {
                    runtime: mode,
                    target: None,
                },
            );
            for (name, _) in runtime_declares(&emit_with(
                USES_THE_RUNTIME,
                &LlvmOptions {
                    runtime: RuntimeMode::Extern,
                    target: None,
                },
            )) {
                let call = format!("@__nyk_{}(", name);
                for line in ll.lines().filter(|l| l.contains(&call)) {
                    assert!(!line.contains("%NykVal %"), "{:?}: {}", mode, line);
                    assert!(
                        line.trim_start().starts_with("call void")
                            || line.starts_with("define")
                            || line.starts_with("declare"),
                        "{:?}: {}",
                        mode,
                        line
                    );
                }
            }
        }
    }

    #[test]
    fn test_target_header() {
        let ll = emit_with(
            "print(1)",
            &LlvmOptions {
                runtime: RuntimeMode::Builtin,
                target: Some(TargetSpec {
                    triple: "x86_64-pc-windows-msvc".to_string(),
                    datalayout: Some("e-m:w-i64:64-n8:16:32:64-S128".to_string()),
                }),
            },
        );
        let header: Vec<&str> = ll.lines().take(3).collect();
        assert_eq!(
            header[1..],
            [
                "target datalayout = \"e-m:w-i64:64-n8:16:32:64-S128\"",
                "target triple = \"x86_64-pc-windows-msvc\""
            ]
        );
        assert!(!emit("print(1)").contains("target triple"));
    }

    /// The body of the LLVM function `name`, from `define` to its `}`.
    fn function_body<'a>(ll: &'a str, name: &str) -> &'a str {
        let start = ll
            .find(&format!("@{}(", name))
            .and_then(|i| ll[..i].rfind("define "))
            .expect("function not found");
        let end = ll[start..].find("\n}\n").expect("unterminated function") + start;
        &ll[start..end]
    }

    #[test]
    fn test_call_dyn_gets_its_arguments_in_an_entry_block_array() {
        let ll = emit("local v = Vector3.new(8, 1, 8)\nlocal m = require(\"@pkg/janis/khione\")");
        let main = function_body(&ll, "nyk_main");
        let entry = &main[..main.find("bb0:").expect("no bb0")];
        assert!(entry.contains("alloca [3 x %NykVal]"), "{}", main);
        assert!(entry.contains("alloca [1 x %NykVal]"), "{}", main);
        assert!(main.contains(", i32 3, ptr %t"), "{}", main);
        assert!(main.contains(", i32 1, ptr %t"), "{}", main);
        assert!(!main.contains("ptr null)"), "{}", main);
    }
}
