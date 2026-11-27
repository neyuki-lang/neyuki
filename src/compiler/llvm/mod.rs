//! Textual LLVM IR backend lowering from [`MirProgram`].

pub mod codegen;
pub mod emit;
pub mod runtime;
pub mod types;

pub use emit::emit_llvm_ir;

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
        let stmts = compile_source(code).expect("syntax error");
        let hir = lower_ast(&stmts, None, OptLevel::O1);
        let thir = lower_hir_to_thir(&hir);
        let mut mir = lower_thir_to_mir(&thir, false);
        optimize_mir(&mut mir, OptLevel::O1);
        emit_llvm_ir(&mir, OptLevel::O1)
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
