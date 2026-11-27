//! Lowers CFG-based [`MirBody`] functions into LLVM IR SSA instructions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::ast::op::{BinOp, UnOp};
use crate::compiler::llvm::types::{LlvmTypeHint, format_llvm_cstring, sanitize_ident};
use crate::compiler::mir::{
    BasicBlockId, LocalId, MirBody, MirConst, MirProgram, MirSpreadSource, Operand, Place,
    Projection, Rvalue, StatementKind, TerminatorKind,
};

pub struct LlvmModuleCollector {
    pub string_literals: Vec<(String, String, usize)>,
    string_map: HashMap<String, usize>,
    pub user_globals: BTreeSet<String>,
    pub fn_names: HashMap<(usize, usize), String>,
}

impl LlvmModuleCollector {
    pub fn new() -> Self {
        Self {
            string_literals: Vec::new(),
            string_map: HashMap::new(),
            user_globals: BTreeSet::new(),
            fn_names: HashMap::new(),
        }
    }

    pub fn intern_string(&mut self, s: &str) -> (String, usize) {
        if let Some(&idx) = self.string_map.get(s) {
            let entry = &self.string_literals[idx];
            return (entry.0.clone(), entry.2);
        }
        let idx = self.string_literals.len();
        let sym = format!("@.str.{}", idx);
        let (escaped, len) = format_llvm_cstring(s);
        self.string_literals.push((sym.clone(), escaped, len));
        self.string_map.insert(s.to_string(), idx);
        (sym, len)
    }

    pub fn collect_program(&mut self, prog: &MirProgram) {
        self.collect_body(&prog.main, "nyk_main", 0);
    }

    fn collect_body(&mut self, body: &MirBody, llvm_name: &str, scope_id: usize) {
        for (idx, child) in body.functions.iter().enumerate() {
            let child_name = format!(
                "@nyk_fn_{}_{}_{}",
                scope_id,
                idx,
                sanitize_ident(child.name.as_deref().unwrap_or("anon"))
            );
            self.fn_names.insert((scope_id, idx), child_name.clone());
            self.collect_body(child, &child_name[1..], scope_id * 100 + idx + 1);
        }
        let _ = llvm_name;
        for bb in &body.blocks {
            for stmt in &bb.statements {
                if let StatementKind::SetGlobal { name, .. } = &stmt.kind {
                    self.user_globals.insert(name.clone());
                }
            }
        }
    }
}

impl Default for LlvmModuleCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// An argument of a call into the runtime.
enum RtArg<'s> {
    /// A `%NykVal` register, passed as its tag and raw bits.
    Val(&'s str),
    /// A pointer register or global.
    Ptr(&'s str),
    I32(usize),
}

/// The values a trailing call or `...` produces.
struct Spread {
    /// Whether they are the function's own varargs (see `emit_body`) rather
    /// than the results of a call, which are in the return area.
    in_args: bool,
    /// How many, as an `i32` register.
    count: String,
}

pub struct FunctionEmitter<'a> {
    collector: &'a mut LlvmModuleCollector,
    scope_id: usize,
    next_tmp: usize,
    out: String,
    local_is_print_builtin: HashMap<LocalId, bool>,
    /// Locals that a closure captures. They live in heap cells (see
    /// `emit_body`), and their `%cell_N` slot holds the cell's pointer.
    boxed: HashSet<LocalId>,
    /// For each child function, where each of its upvalues comes from when
    /// a closure of it is made: `None` for a local of this function (named
    /// by the `Closure` rvalue), `Some(i)` for this function's upvalue `i`.
    child_upvalue_sources: Vec<Vec<Option<u8>>>,
    /// `alloca`s that belong in the entry block, spliced in by `emit_body`
    /// so that every stack slot is allocated once per call, where mem2reg
    /// and SROA expect it, rather than at the instruction that needs it.
    entry_allocas: String,
    next_label: usize,
    /// For each numeric `for`, by its base local: the locals of its limit,
    /// step and loop variable, which `ForPrep` names and `ForLoop` does not.
    /// They need not follow the base, as evaluating the limit or the step
    /// may allocate temporaries.
    for_locals: HashMap<LocalId, (LocalId, LocalId, LocalId)>,
}

impl<'a> FunctionEmitter<'a> {
    pub fn new(collector: &'a mut LlvmModuleCollector, scope_id: usize) -> Self {
        Self {
            collector,
            scope_id,
            next_tmp: 0,
            out: String::new(),
            local_is_print_builtin: HashMap::new(),
            boxed: HashSet::new(),
            child_upvalue_sources: Vec::new(),
            entry_allocas: String::new(),
            next_label: 0,
            for_locals: HashMap::new(),
        }
    }

    /// A fresh label suffix for the blocks one call site emits.
    fn label_id(&mut self) -> usize {
        let id = self.next_label;
        self.next_label += 1;
        id
    }

    /// Allocates `ty` in the entry block and returns the pointer register.
    fn entry_alloca(&mut self, ty: &str) -> String {
        let p = self.tmp();
        let _ = writeln!(self.entry_allocas, "  {} = alloca {}", p, ty);
        p
    }

    /// Lowers `args` to the parameter list of a runtime function: a value
    /// becomes its tag and raw bits (see the ABI notes in runtime.rs).
    fn rt_params(&mut self, args: &[RtArg]) -> String {
        let mut params = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                RtArg::Val(v) => {
                    let tag = self.tmp();
                    let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 0", tag, v);
                    let raw = self.tmp();
                    let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 1", raw, v);
                    params.push(format!("i8 zeroext {}, i64 {}", tag, raw));
                }
                RtArg::Ptr(p) => params.push(format!("ptr {}", p)),
                RtArg::I32(n) => params.push(format!("i32 {}", n)),
            }
        }
        params.join(", ")
    }

    /// Calls the runtime function `func`, which returns a value through an
    /// out-pointer, and returns the register holding that value.
    fn rt_call(&mut self, func: &str, args: &[RtArg]) -> String {
        let params = self.rt_params(args);
        let slot = self.entry_alloca("%NykVal");
        let sep = if params.is_empty() { "" } else { ", " };
        let _ = writeln!(
            self.out,
            "  call void @{}(ptr {}{}{})",
            func, slot, sep, params
        );
        let res = self.tmp();
        let _ = writeln!(self.out, "  {} = load %NykVal, ptr {}", res, slot);
        res
    }

    /// Calls the runtime function `func`, which returns nothing.
    fn rt_call_void(&mut self, func: &str, args: &[RtArg]) {
        let params = self.rt_params(args);
        let _ = writeln!(self.out, "  call void @{}({})", func, params);
    }

    /// Like [`Self::rt_call`], for a runtime function that returns a pointer.
    fn rt_call_ptr(&mut self, func: &str, args: &[RtArg]) -> String {
        let params = self.rt_params(args);
        let res = self.tmp();
        let _ = writeln!(self.out, "  {} = call ptr @{}({})", res, func, params);
        res
    }

    /// Calls `callee` with the `argc` arguments at `argv` (see "Calls and
    /// multiple values" in runtime.rs) and returns the register holding the
    /// number of results, which are in the return area. A function value
    /// (tag 6) is called directly through its pointer, the `print` builtin
    /// (tag 7) prints, and anything else goes to `__nyk_call_dyn`, for the
    /// runtime to call natives, tables with `__call` and so on.
    fn emit_call(&mut self, callee: &str, argv: &str, argc: &str) -> String {
        let id = self.label_id();
        let lbl_direct = format!("call_dir_{}", id);
        let lbl_dyn = format!("call_dyn_{}", id);
        let lbl_prt = format!("call_prt_{}", id);
        let lbl_ext = format!("call_ext_{}", id);
        let lbl_join = format!("call_join_{}", id);

        let tag = self.tmp();
        let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 0", tag, callee);
        let is_fn = self.tmp();
        let _ = writeln!(self.out, "  {} = icmp eq i8 {}, 6", is_fn, tag);
        let _ = writeln!(
            self.out,
            "  br i1 {}, label %{}, label %{}",
            is_fn, lbl_direct, lbl_dyn
        );

        // Direct function-pointer invocation path
        let _ = writeln!(self.out, "{}:", lbl_direct);
        let raw_ptr = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = extractvalue %NykVal {}, 1",
            raw_ptr, callee
        );
        // A function value points to its closure object, whose first word is
        // the code; the object itself is the callee's hidden `%env`.
        let env = self.tmp();
        let _ = writeln!(self.out, "  {} = inttoptr i64 {} to ptr", env, raw_ptr);
        let fn_ptr = self.tmp();
        let _ = writeln!(self.out, "  {} = load ptr, ptr {}", fn_ptr, env);
        let direct_n = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = call i32 {}(ptr {}, ptr {}, i32 {})",
            direct_n, fn_ptr, env, argv, argc
        );
        let _ = writeln!(self.out, "  br label %{}", lbl_join);

        // Dynamic fallback path: prints if tag == 7, else __nyk_call_dyn
        let _ = writeln!(self.out, "{}:", lbl_dyn);
        let is_prt = self.tmp();
        let _ = writeln!(self.out, "  {} = icmp eq i8 {}, 7", is_prt, tag);
        let _ = writeln!(
            self.out,
            "  br i1 {}, label %{}, label %{}",
            is_prt, lbl_prt, lbl_ext
        );

        let _ = writeln!(self.out, "{}:", lbl_prt);
        let _ = writeln!(
            self.out,
            "  call void @__nyk_print_list(ptr {}, i32 {})",
            argv, argc
        );
        let _ = writeln!(self.out, "  br label %{}", lbl_join);

        let _ = writeln!(self.out, "{}:", lbl_ext);
        let params = self.rt_params(&[RtArg::Val(callee)]);
        let dyn_n = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = call i32 @__nyk_call_dyn({}, i32 {}, ptr {})",
            dyn_n, params, argc, argv
        );
        let _ = writeln!(self.out, "  br label %{}", lbl_join);

        let _ = writeln!(self.out, "{}:", lbl_join);
        let merged = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = phi i32 [ {}, %{} ], [ 0, %{} ], [ {}, %{} ]",
            merged, direct_n, lbl_direct, lbl_prt, dyn_n, lbl_ext
        );
        merged
    }

    /// Stores `args` into a `[N x %NykVal]` array allocated in the entry
    /// block and returns a pointer to it (`null` when there are none).
    fn emit_argv(&mut self, args: &[String]) -> String {
        if args.is_empty() {
            return "null".to_string();
        }
        let ty = format!("[{} x %NykVal]", args.len());
        let argv = self.entry_alloca(&ty);
        self.store_values(&argv, args);
        argv
    }

    fn emit_print(&mut self, args: &[String]) {
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str("  call void @__nyk_print_sp()\n");
            }
            let _ = writeln!(self.out, "  call void @__nyk_print_one(%NykVal {})", a);
        }
        self.out.push_str("  call void @__nyk_print_nl()\n");
    }

    /// The size in bytes of a `%NykVal`, as a register.
    fn val_size(&mut self) -> String {
        let end = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = getelementptr %NykVal, ptr null, i64 1",
            end
        );
        let size = self.tmp();
        let _ = writeln!(self.out, "  {} = ptrtoint ptr {} to i64", size, end);
        size
    }

    /// Evaluates a spread source: runs the call, or reads `...`.
    fn eval_spread(&mut self, source: &MirSpreadSource) -> Spread {
        match source {
            MirSpreadSource::Call {
                callee,
                args,
                trailing,
            } => {
                let (callee, argv, argc, saved_sp) =
                    self.eval_call_parts(callee, args, trailing.as_deref());
                let count = self.emit_call(&callee, &argv, &argc);
                self.restore_stack(saved_sp);
                Spread {
                    in_args: false,
                    count,
                }
            }
            MirSpreadSource::Vararg => Spread {
                in_args: true,
                count: "%va_n".to_string(),
            },
        }
    }

    /// A pointer to the values of `spread`, valid until the next call.
    fn spread_ptr(&mut self, spread: &Spread) -> String {
        if spread.in_args {
            "%va_base".to_string()
        } else {
            self.rt_call_ptr("__nyk_ret_buf", &[RtArg::I32(0)])
        }
    }

    /// Undoes the stack allocation of a call's argument array, if it made one.
    fn restore_stack(&mut self, saved_sp: Option<String>) {
        if let Some(sp) = saved_sp {
            let _ = writeln!(self.out, "  call void @llvm.stackrestore.p0(ptr {})", sp);
        }
    }

    /// Evaluates a callee and its arguments in order, then a `trailing`
    /// spread source, whose values all become arguments. Returns the callee,
    /// the argument array, the argument count (a number or a register) and,
    /// when the array was allocated on the stack, the stack pointer to
    /// restore after the call.
    fn eval_call_parts(
        &mut self,
        callee: &Operand,
        args: &[Operand],
        trailing: Option<&MirSpreadSource>,
    ) -> (String, String, String, Option<String>) {
        let c_val = self.eval_operand(callee);
        let arg_vals: Vec<String> = args.iter().map(|a| self.eval_operand(a)).collect();
        let Some(src) = trailing else {
            let argv = self.emit_argv(&arg_vals);
            return (c_val, argv, arg_vals.len().to_string(), None);
        };
        let spread = self.eval_spread(src);
        let src_ptr = self.spread_ptr(&spread);
        // The number of arguments is only known now, so the array is
        // allocated here and freed after the call.
        let sp = self.tmp();
        let _ = writeln!(self.out, "  {} = call ptr @llvm.stacksave.p0()", sp);
        let fixed = arg_vals.len();
        let total = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = add i32 {}, {}",
            total, fixed, spread.count
        );
        let total_z = self.tmp();
        let _ = writeln!(self.out, "  {} = zext i32 {} to i64", total_z, total);
        let argv = self.tmp();
        let _ = writeln!(self.out, "  {} = alloca %NykVal, i64 {}", argv, total_z);
        self.store_values(&argv, &arg_vals);
        self.copy_values(&argv, fixed, &src_ptr, &spread.count);
        (c_val, argv, total, Some(sp))
    }

    /// Stores `vals` as the first elements of the value array `base`.
    fn store_values(&mut self, base: &str, vals: &[String]) {
        for (i, v) in vals.iter().enumerate() {
            let slot = self.tmp();
            let _ = writeln!(
                self.out,
                "  {} = getelementptr %NykVal, ptr {}, i64 {}",
                slot, base, i
            );
            let _ = writeln!(self.out, "  store %NykVal {}, ptr {}", v, slot);
        }
    }

    /// Copies the `count` values at `src` to the value array `base`, from
    /// element `at` on. The arrays may overlap.
    fn copy_values(&mut self, base: &str, at: usize, src: &str, count: &str) {
        let dst = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = getelementptr %NykVal, ptr {}, i64 {}",
            dst, base, at
        );
        let size = self.val_size();
        let count_z = self.tmp();
        let _ = writeln!(self.out, "  {} = zext i32 {} to i64", count_z, count);
        let bytes = self.tmp();
        let _ = writeln!(self.out, "  {} = mul i64 {}, {}", bytes, count_z, size);
        let _ = writeln!(
            self.out,
            "  call void @llvm.memmove.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)",
            dst, src, bytes
        );
    }

    /// Reads the first `want` results of the call that returned `count`
    /// results, nil where there are fewer.
    fn read_results(&mut self, count: &str, want: usize) -> Vec<String> {
        if want == 0 {
            return Vec::new();
        }
        let area = self.rt_call_ptr("__nyk_ret_buf", &[RtArg::I32(0)]);
        (0..want).map(|i| self.list_get(&area, count, i)).collect()
    }

    /// Element `i` of the `n` values at `ptr`, nil past the end.
    fn list_get(&mut self, ptr: &str, n: &str, i: usize) -> String {
        let v = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = call %NykVal @__nyk_list_get(ptr {}, i32 {}, i32 {})",
            v, ptr, n, i
        );
        v
    }

    /// Stores each of `vals` in the local of the same position of `dsts`,
    /// nil in the locals past the end of `vals`.
    fn store_call_results(&mut self, dsts: &[LocalId], vals: &[String]) {
        for (i, dst) in dsts.iter().enumerate() {
            let v = match vals.get(i) {
                Some(v) => v.clone(),
                None => {
                    let nil = self.tmp();
                    let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_nil()", nil);
                    nil
                }
            };
            self.store_local(*dst, &v);
        }
    }

    /// Returns `vals`, followed by the `spread` values if there is one: the
    /// results go to the return area and the function returns their count.
    fn emit_return(&mut self, vals: &[String], spread: Option<&Spread>) {
        let Some(spread) = spread else {
            let n = vals.len();
            if n > 0 {
                let area = self.rt_call_ptr("__nyk_ret_buf", &[RtArg::I32(n)]);
                self.store_values(&area, vals);
            }
            let _ = writeln!(self.out, "  ret i32 {}", n);
            return;
        };
        let total = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = add i32 {}, {}",
            total,
            vals.len(),
            spread.count
        );
        // Reserving room may move the area, and with it the results of a
        // call, so the values to copy are taken from the new area.
        let area = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = call ptr @__nyk_ret_buf(i32 {})",
            area, total
        );
        let src = if spread.in_args {
            "%va_base".to_string()
        } else {
            area.clone()
        };
        self.copy_values(&area, vals.len(), &src, &spread.count);
        self.store_values(&area, vals);
        let _ = writeln!(self.out, "  ret i32 {}", total);
    }

    fn tmp(&mut self) -> String {
        let id = self.next_tmp;
        self.next_tmp += 1;
        format!("%t{}", id)
    }

    fn slot(id: LocalId) -> String {
        format!("%slot_{}", id.0)
    }

    fn cell_slot(id: LocalId) -> String {
        format!("%cell_{}", id.0)
    }

    /// The address that holds local `id`'s value: its stack slot, or for a
    /// captured local the heap cell its `%cell_N` slot points to now.
    fn slot_ptr(&mut self, id: LocalId) -> String {
        if !self.boxed.contains(&id) {
            return Self::slot(id);
        }
        let p = self.tmp();
        let _ = writeln!(self.out, "  {} = load ptr, ptr {}", p, Self::cell_slot(id));
        p
    }

    /// Stores `val` into local `id`.
    fn store_local(&mut self, id: LocalId, val: &str) {
        let p = self.slot_ptr(id);
        let _ = writeln!(self.out, "  store %NykVal {}, ptr {}", val, p);
    }

    /// Allocates a fresh cell holding `val` and returns its pointer.
    fn new_cell(&mut self, val: &str) -> String {
        let tag = self.tmp();
        let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 0", tag, val);
        let raw = self.tmp();
        let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 1", raw, val);
        let cell = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = call ptr @__nyk_cell_new(i8 zeroext {}, i64 {})",
            cell, tag, raw
        );
        cell
    }

    fn bb_name(id: BasicBlockId) -> String {
        format!("bb{}", id.0)
    }

    fn emit_string_ptr(&mut self, s: &str) -> String {
        let (sym, len) = self.collector.intern_string(s);
        let p = self.tmp();
        let _ = writeln!(
            self.out,
            "  {} = getelementptr inbounds [{} x i8], ptr {}, i64 0, i64 0",
            p, len, sym
        );
        p
    }

    fn eval_const(&mut self, c: &MirConst) -> String {
        let dst = self.tmp();
        match c {
            MirConst::Nil => {
                let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_nil()", dst);
            }
            MirConst::Bool(b) => {
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @__nyk_mk_bool(i1 {})",
                    dst, b
                );
            }
            MirConst::Int(bi) => {
                let v = wrap_to_i64(bi);
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @__nyk_mk_int(i64 {})",
                    dst, v
                );
            }
            MirConst::Float(f) => {
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @__nyk_mk_float(double 0x{:016X})",
                    dst,
                    f.to_bits()
                );
            }
            MirConst::String(s) => {
                let ptr = self.emit_string_ptr(s);
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @__nyk_mk_str(ptr {})",
                    dst, ptr
                );
            }
        }
        dst
    }

    fn eval_place(&mut self, place: &Place) -> String {
        let src = self.slot_ptr(place.local);
        let mut cur = self.tmp();
        let _ = writeln!(self.out, "  {} = load %NykVal, ptr {}", cur, src);
        for proj in &place.projections {
            let key = match proj {
                Projection::Field(f) => {
                    let s = self.emit_string_ptr(f);
                    let k = self.tmp();
                    let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_str(ptr {})", k, s);
                    k
                }
                Projection::Index(idx) => self.eval_operand(idx),
            };
            cur = self.rt_call("__nyk_table_get", &[RtArg::Val(&cur), RtArg::Val(&key)]);
        }
        cur
    }

    fn store_place(&mut self, place: &Place, val_reg: &str) {
        if place.projections.is_empty() {
            self.store_local(place.local, val_reg);
            return;
        }
        let src = self.slot_ptr(place.local);
        let mut cur = self.tmp();
        let _ = writeln!(self.out, "  {} = load %NykVal, ptr {}", cur, src);
        let last = place.projections.len() - 1;
        for proj in &place.projections[..last] {
            let key = match proj {
                Projection::Field(f) => {
                    let s = self.emit_string_ptr(f);
                    let k = self.tmp();
                    let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_str(ptr {})", k, s);
                    k
                }
                Projection::Index(idx) => self.eval_operand(idx),
            };
            cur = self.rt_call("__nyk_table_get", &[RtArg::Val(&cur), RtArg::Val(&key)]);
        }
        let last_key = match &place.projections[last] {
            Projection::Field(f) => {
                let s = self.emit_string_ptr(f);
                let k = self.tmp();
                let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_str(ptr {})", k, s);
                k
            }
            Projection::Index(idx) => self.eval_operand(idx),
        };
        self.rt_call_void(
            "__nyk_table_set",
            &[RtArg::Val(&cur), RtArg::Val(&last_key), RtArg::Val(val_reg)],
        );
    }

    fn eval_operand(&mut self, op: &Operand) -> String {
        match op {
            Operand::Constant(c) => self.eval_const(c),
            Operand::Copy(p) | Operand::Move(p) => self.eval_place(p),
        }
    }

    fn eval_rvalue(&mut self, rv: &Rvalue, dst_local: Option<LocalId>) -> String {
        match rv {
            Rvalue::Use(op) => {
                if let (Some(dst), Operand::Copy(p) | Operand::Move(p)) = (dst_local, op)
                    && p.projections.is_empty()
                {
                    let is_p = self
                        .local_is_print_builtin
                        .get(&p.local)
                        .copied()
                        .unwrap_or(false);
                    self.local_is_print_builtin.insert(dst, is_p);
                }
                self.eval_operand(op)
            }
            Rvalue::BinaryOp { op, lhs, rhs } => {
                let l = self.eval_operand(lhs);
                let r = self.eval_operand(rhs);
                if *op == BinOp::Concat {
                    return self.rt_call("__nyk_concat", &[RtArg::Val(&l), RtArg::Val(&r)]);
                }
                let res = self.tmp();
                let helper = match op {
                    BinOp::Add => "__nyk_add",
                    BinOp::Sub => "__nyk_sub",
                    BinOp::Mul => "__nyk_mul",
                    BinOp::Div => "__nyk_div",
                    BinOp::IDiv => "__nyk_idiv",
                    BinOp::Mod => "__nyk_mod",
                    BinOp::BitAnd => "__nyk_bitand",
                    BinOp::BitOr => "__nyk_bitor",
                    BinOp::BitXor => "__nyk_bitxor",
                    BinOp::Shl => "__nyk_shl",
                    BinOp::Shr => "__nyk_shr",
                    BinOp::LShl => "__nyk_lshl",
                    BinOp::LShr => "__nyk_lshr",
                    BinOp::Pow => "__nyk_pow",
                    BinOp::Concat => unreachable!("handled above"),
                    BinOp::Eq => "__nyk_eq",
                    BinOp::Ne => "__nyk_ne",
                    BinOp::Lt => "__nyk_lt",
                    BinOp::Le => "__nyk_le",
                    BinOp::Gt => "__nyk_gt",
                    BinOp::Ge => "__nyk_ge",
                    // MIR lowers the short-circuit operators to branches, so
                    // they never reach a `BinaryOp`.
                    BinOp::And | BinOp::Or | BinOp::Coalesce => "__nyk_add",
                };
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @{}(%NykVal {}, %NykVal {})",
                    res, helper, l, r
                );
                res
            }
            Rvalue::UnaryOp { op, src } => {
                let s = self.eval_operand(src);
                if *op == UnOp::Len {
                    return self.rt_call("__nyk_len", &[RtArg::Val(&s)]);
                }
                let res = self.tmp();
                let helper = match op {
                    UnOp::Neg => "__nyk_neg",
                    UnOp::Not => "__nyk_not",
                    UnOp::Len => unreachable!("handled above"),
                    UnOp::BitNot => "__nyk_bitnot",
                };
                let _ = writeln!(
                    self.out,
                    "  {} = call %NykVal @{}(%NykVal {})",
                    res, helper, s
                );
                res
            }
            Rvalue::Global(name) => {
                if name == "print" {
                    if let Some(dst) = dst_local {
                        self.local_is_print_builtin.insert(dst, true);
                    }
                    let res = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = insertvalue %NykVal {{ i8 7, i64 0 }}, i64 0, 1",
                        res
                    );
                    return res;
                }
                if let Some(dst) = dst_local {
                    self.local_is_print_builtin.insert(dst, false);
                }
                if self.collector.user_globals.contains(name) {
                    let gsym = format!("@__nyk_glob_{}", sanitize_ident(name));
                    let res = self.tmp();
                    let _ = writeln!(self.out, "  {} = load %NykVal, ptr {}", res, gsym);
                    res
                } else {
                    let s = self.emit_string_ptr(name);
                    self.rt_call("__nyk_global_get", &[RtArg::Ptr(&s)])
                }
            }
            Rvalue::Upvalue(idx) => self.rt_call(
                "__nyk_upval_get",
                &[RtArg::Ptr("%env"), RtArg::I32(*idx as usize)],
            ),
            Rvalue::ModuleImport { module, field } => {
                let mp = self.emit_string_ptr(module);
                let fp = self.emit_string_ptr(field);
                self.rt_call("__nyk_import_get", &[RtArg::Ptr(&mp), RtArg::Ptr(&fp)])
            }
            Rvalue::NewTable {
                array_cap,
                fields_cap,
            } => self.rt_call(
                "__nyk_table_new",
                &[
                    RtArg::I32(*array_cap as usize),
                    RtArg::I32(*fields_cap as usize),
                ],
            ),
            Rvalue::Closure { fn_idx, captures } => {
                let sym = self
                    .collector
                    .fn_names
                    .get(&(self.scope_id, *fn_idx))
                    .cloned()
                    .unwrap_or_else(|| "@nyk_unknown".to_string());
                if captures.is_empty() {
                    // A function that captures nothing has one static
                    // closure object.
                    let res = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_mk_fn(ptr {}.closure)",
                        res, sym
                    );
                    return res;
                }
                // The cells to share: a captured local's own cell, or the one
                // behind an upvalue of this function.
                let ty = format!("[{} x ptr]", captures.len());
                let caps = self.entry_alloca(&ty);
                for (i, cap) in captures.iter().enumerate() {
                    let cell = match cap {
                        Some(local) => {
                            let c = self.tmp();
                            let _ = writeln!(
                                self.out,
                                "  {} = load ptr, ptr {}",
                                c,
                                Self::cell_slot(*local)
                            );
                            c
                        }
                        None => {
                            let up = self
                                .child_upvalue_sources
                                .get(*fn_idx)
                                .and_then(|srcs| srcs.get(i).copied().flatten())
                                .unwrap_or(0);
                            let c = self.tmp();
                            let _ = writeln!(
                                self.out,
                                "  {} = call ptr @__nyk_upval_cell(ptr %env, i32 {})",
                                c, up
                            );
                            c
                        }
                    };
                    let at = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = getelementptr inbounds {}, ptr {}, i64 0, i64 {}",
                        at, ty, caps, i
                    );
                    let _ = writeln!(self.out, "  store ptr {}, ptr {}", cell, at);
                }
                self.rt_call(
                    "__nyk_closure",
                    &[
                        RtArg::Ptr(&sym),
                        RtArg::I32(captures.len()),
                        RtArg::Ptr(&caps),
                    ],
                )
            }
            Rvalue::Vararg => self.list_get("%va_base", "%va_n", 0),
        }
    }

    fn is_print_operand(&self, op: &Operand) -> bool {
        match op {
            Operand::Copy(p) | Operand::Move(p) if p.projections.is_empty() => self
                .local_is_print_builtin
                .get(&p.local)
                .copied()
                .unwrap_or(false),
            _ => false,
        }
    }

    pub fn emit_body(mut self, body: &MirBody, llvm_name: &str) -> String {
        // A local that a closure captures is shared with it, so it lives in
        // a heap cell instead of a stack slot.
        for bb in &body.blocks {
            for stmt in &bb.statements {
                if let StatementKind::Assign(_, Rvalue::Closure { captures, .. }) = &stmt.kind {
                    self.boxed.extend(captures.iter().flatten().copied());
                }
            }
        }
        for bb in &body.blocks {
            if let TerminatorKind::ForPrep {
                base,
                limit,
                step,
                loop_var,
                ..
            } = &bb.terminator
            {
                self.for_locals.insert(*base, (*limit, *step, *loop_var));
            }
        }
        self.child_upvalue_sources = body
            .functions
            .iter()
            .map(|child| {
                child
                    .upvalues
                    .iter()
                    .map(|u| if u.in_stack { None } else { Some(u.index) })
                    .collect()
            })
            .collect();
        let mut child_defs = String::new();
        for (idx, child) in body.functions.iter().enumerate() {
            let child_name = self
                .collector
                .fn_names
                .get(&(self.scope_id, idx))
                .cloned()
                .unwrap_or_else(|| format!("@nyk_fn_{}_{}", self.scope_id, idx));
            let child_emitter = FunctionEmitter::new(self.collector, self.scope_id * 100 + idx + 1);
            child_defs.push_str(&child_emitter.emit_body(child, &child_name[1..]));
            child_defs.push('\n');
        }

        // Every function takes its closure object as a hidden `%env` (see
        // `emit_call`); one that captures nothing uses this static object.
        let _ = writeln!(
            self.out,
            "@{}.closure = internal constant [1 x ptr] [ptr @{}]\n",
            llvm_name, llvm_name
        );
        let _ = writeln!(
            self.out,
            "define i32 @{}(ptr %env, ptr %args, i32 %argc) {{",
            llvm_name
        );
        self.out.push_str("entry:\n");
        let entry_pos = self.out.len();
        let num_params = body.num_params as usize;
        if body.is_vararg {
            // `...` is what follows the named parameters in the arguments.
            let _ = writeln!(
                self.out,
                "  %va_more = icmp ugt i32 %argc, {np}\n  %va_diff = sub i32 %argc, {np}\n  \
                 %va_n = select i1 %va_more, i32 %va_diff, i32 0\n  \
                 %va_base = getelementptr %NykVal, ptr %args, i64 {np}",
                np = num_params
            );
        }

        // Allocate stack slots for all MIR locals so mem2reg/O1 can promote them cleanly.
        for decl in &body.locals {
            let hint = LlvmTypeHint::from_neyuki(&decl.ty).ir_comment_name();
            let name = decl.name.as_deref().unwrap_or("tmp");
            let zero = self.tmp();
            if self.boxed.contains(&decl.id) {
                let _ = writeln!(
                    self.out,
                    "  {} = alloca ptr ; cell of {} {}",
                    Self::cell_slot(decl.id),
                    hint,
                    name
                );
                let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_nil()", zero);
                let cell = self.new_cell(&zero);
                let _ = writeln!(
                    self.out,
                    "  store ptr {}, ptr {}",
                    cell,
                    Self::cell_slot(decl.id)
                );
                continue;
            }
            let _ = writeln!(
                self.out,
                "  {} = alloca %NykVal ; {} {}",
                Self::slot(decl.id),
                hint,
                name
            );
            let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_nil()", zero);
            let _ = writeln!(
                self.out,
                "  store %NykVal {}, ptr {}",
                zero,
                Self::slot(decl.id)
            );
        }

        // Bind incoming arguments to their corresponding argument locals;
        // a missing one is nil.
        let mut arg_idx = 0usize;
        for decl in &body.locals {
            if matches!(decl.kind, crate::compiler::mir::LocalKind::Arg) && arg_idx < num_params {
                let arg = self.list_get("%args", "%argc", arg_idx);
                self.store_local(decl.id, &arg);
                arg_idx += 1;
            }
        }

        if let Some(first_bb) = body.blocks.first() {
            let _ = writeln!(self.out, "  br label %{}", Self::bb_name(first_bb.id));
        } else {
            self.out.push_str("  ret i32 0\n");
        }

        for bb in &body.blocks {
            let _ = writeln!(self.out, "{}:", Self::bb_name(bb.id));
            for stmt in &bb.statements {
                match &stmt.kind {
                    StatementKind::Assign(place, rvalue) => {
                        let dst_local = if place.projections.is_empty() {
                            Some(place.local)
                        } else {
                            None
                        };
                        let val = self.eval_rvalue(rvalue, dst_local);
                        self.store_place(place, &val);
                    }
                    StatementKind::SetGlobal { name, value } => {
                        let val = self.eval_operand(value);
                        if self.collector.user_globals.contains(name) {
                            let gsym = format!("@__nyk_glob_{}", sanitize_ident(name));
                            let _ = writeln!(self.out, "  store %NykVal {}, ptr {}", val, gsym);
                        } else {
                            let s = self.emit_string_ptr(name);
                            self.rt_call_void(
                                "__nyk_global_set",
                                &[RtArg::Ptr(&s), RtArg::Val(&val)],
                            );
                        }
                    }
                    StatementKind::SetUpval { index, value } => {
                        let val = self.eval_operand(value);
                        self.rt_call_void(
                            "__nyk_upval_set",
                            &[
                                RtArg::Ptr("%env"),
                                RtArg::I32(*index as usize),
                                RtArg::Val(&val),
                            ],
                        );
                    }
                    StatementKind::AppendArray { table, value } => {
                        let t = self.eval_place(&Place::local(*table));
                        let v = self.eval_operand(value);
                        self.rt_call_void("__nyk_table_append", &[RtArg::Val(&t), RtArg::Val(&v)]);
                    }
                    StatementKind::VarargMany(dsts) => {
                        let vals: Vec<String> = (0..dsts.len())
                            .map(|i| self.list_get("%va_base", "%va_n", i))
                            .collect();
                        self.store_call_results(dsts, &vals);
                    }
                    StatementKind::CloseUpvals(locals) => {
                        // The end of a loop iteration: closures made in it keep
                        // the current cell, and the next iteration's variable
                        // gets a fresh one (with the same value until it is
                        // assigned).
                        for local in locals {
                            if !self.boxed.contains(local) {
                                continue;
                            }
                            let cur = self.slot_ptr(*local);
                            let val = self.tmp();
                            let _ = writeln!(self.out, "  {} = load %NykVal, ptr {}", val, cur);
                            let cell = self.new_cell(&val);
                            let _ = writeln!(
                                self.out,
                                "  store ptr {}, ptr {}",
                                cell,
                                Self::cell_slot(*local)
                            );
                        }
                    }
                    StatementKind::DebugLocal { .. } | StatementKind::DebugScopeEnd { .. } => {}
                }
            }

            match &bb.terminator {
                TerminatorKind::Goto(target) => {
                    let _ = writeln!(self.out, "  br label %{}", Self::bb_name(*target));
                }
                TerminatorKind::SwitchBool {
                    cond,
                    true_bb,
                    false_bb,
                } => {
                    let c = self.eval_operand(cond);
                    let t = self.tmp();
                    let _ = writeln!(self.out, "  {} = call i1 @__nyk_truthy(%NykVal {})", t, c);
                    let _ = writeln!(
                        self.out,
                        "  br i1 {}, label %{}, label %{}",
                        t,
                        Self::bb_name(*true_bb),
                        Self::bb_name(*false_bb)
                    );
                }
                TerminatorKind::Call {
                    dsts,
                    callee,
                    args,
                    target,
                    ..
                } => {
                    if self.is_print_operand(callee) {
                        let arg_vals: Vec<String> =
                            args.iter().map(|a| self.eval_operand(a)).collect();
                        self.emit_print(&arg_vals);
                        self.store_call_results(dsts, &[]);
                    } else {
                        let (c_val, argv, argc, _) = self.eval_call_parts(callee, args, None);
                        let n = self.emit_call(&c_val, &argv, &argc);
                        let vals = self.read_results(&n, dsts.len());
                        self.store_call_results(dsts, &vals);
                    }
                    let _ = writeln!(self.out, "  br label %{}", Self::bb_name(*target));
                }
                TerminatorKind::SpreadCall {
                    dsts,
                    callee,
                    fixed_args,
                    trailing,
                    target,
                    ..
                } => {
                    let (c_val, argv, argc, saved_sp) =
                        self.eval_call_parts(callee, fixed_args, Some(trailing));
                    let n = self.emit_call(&c_val, &argv, &argc);
                    self.restore_stack(saved_sp);
                    let vals = self.read_results(&n, dsts.len());
                    self.store_call_results(dsts, &vals);
                    let _ = writeln!(self.out, "  br label %{}", Self::bb_name(*target));
                }
                TerminatorKind::SpreadList {
                    table,
                    source,
                    target,
                } => {
                    let t = self.eval_place(&Place::local(*table));
                    let spread = self.eval_spread(source);
                    let src = self.spread_ptr(&spread);
                    let _ = writeln!(
                        self.out,
                        "  call void @__nyk_append_list(%NykVal {}, ptr {}, i32 {})",
                        t, src, spread.count
                    );
                    let _ = writeln!(self.out, "  br label %{}", Self::bb_name(*target));
                }
                TerminatorKind::SpreadReturn { values, source } => {
                    let vals: Vec<String> = values.iter().map(|v| self.eval_operand(v)).collect();
                    let spread = self.eval_spread(source);
                    self.emit_return(&vals, Some(&spread));
                }
                TerminatorKind::ForPrep {
                    base,
                    step,
                    test_bb,
                    ..
                } => {
                    let b = self.eval_place(&Place::local(*base));
                    let s = self.eval_place(&Place::local(*step));
                    let init = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_sub(%NykVal {}, %NykVal {})",
                        init, b, s
                    );
                    self.store_local(*base, &init);
                    let _ = writeln!(self.out, "  br label %{}", Self::bb_name(*test_bb));
                }
                TerminatorKind::ForLoop {
                    base,
                    body_bb,
                    exit_bb,
                } => {
                    let (limit_local, step_local, loop_var_local) = self.for_locals[base];

                    let b = self.eval_place(&Place::local(*base));
                    let lim = self.eval_place(&Place::local(limit_local));
                    let stp = self.eval_place(&Place::local(step_local));

                    let next = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_add(%NykVal {}, %NykVal {})",
                        next, b, stp
                    );
                    self.store_local(*base, &next);
                    self.store_local(loop_var_local, &next);

                    // Positive step: next <= limit; negative step: limit <= next.
                    let zero = self.tmp();
                    let _ = writeln!(self.out, "  {} = call %NykVal @__nyk_mk_int(i64 0)", zero);
                    let is_neg_v = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_lt(%NykVal {}, %NykVal {})",
                        is_neg_v, stp, zero
                    );
                    let is_neg = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call i1 @__nyk_truthy(%NykVal {})",
                        is_neg, is_neg_v
                    );

                    let pos_cmp_v = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_le(%NykVal {}, %NykVal {})",
                        pos_cmp_v, next, lim
                    );
                    let neg_cmp_v = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call %NykVal @__nyk_ge(%NykVal {}, %NykVal {})",
                        neg_cmp_v, next, lim
                    );

                    let cond_v = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = select i1 {}, %NykVal {}, %NykVal {}",
                        cond_v, is_neg, neg_cmp_v, pos_cmp_v
                    );
                    let cond = self.tmp();
                    let _ = writeln!(
                        self.out,
                        "  {} = call i1 @__nyk_truthy(%NykVal {})",
                        cond, cond_v
                    );
                    let _ = writeln!(
                        self.out,
                        "  br i1 {}, label %{}, label %{}",
                        cond,
                        Self::bb_name(*body_bb),
                        Self::bb_name(*exit_bb)
                    );
                }
                TerminatorKind::TForStep {
                    base,
                    state,
                    ctrl,
                    vars,
                    body_bb,
                    exit_bb,
                } => {
                    // vars = base(state, ctrl); stop on nil, else ctrl = vars[0].
                    let iter_fn = self.eval_place(&Place::local(*base));
                    let st = self.eval_place(&Place::local(*state));
                    let ct = self.eval_place(&Place::local(*ctrl));
                    let argv = self.emit_argv(&[st, ct]);
                    let n = self.emit_call(&iter_fn, &argv, "2");
                    let vals = self.read_results(&n, vars.len().max(1));
                    let first = vals[0].clone();
                    self.store_call_results(vars, &vals);
                    self.store_local(*ctrl, &first);
                    let tag = self.tmp();
                    let _ = writeln!(self.out, "  {} = extractvalue %NykVal {}, 0", tag, first);
                    let is_nil = self.tmp();
                    let _ = writeln!(self.out, "  {} = icmp eq i8 {}, 0", is_nil, tag);
                    let _ = writeln!(
                        self.out,
                        "  br i1 {}, label %{}, label %{}",
                        is_nil,
                        Self::bb_name(*exit_bb),
                        Self::bb_name(*body_bb)
                    );
                }
                TerminatorKind::Return(ops) => {
                    // A bare `return` gives one nil, as in the VM.
                    let vals: Vec<String> = if ops.is_empty() {
                        vec!["zeroinitializer".to_string()]
                    } else {
                        ops.iter().map(|o| self.eval_operand(o)).collect()
                    };
                    self.emit_return(&vals, None);
                }
                TerminatorKind::Unreachable => {
                    self.emit_return(&["zeroinitializer".to_string()], None);
                }
            }
        }

        self.out.push_str("}\n");
        self.out.insert_str(entry_pos, &self.entry_allocas);

        let mut combined = child_defs;
        combined.push_str(&self.out);
        combined
    }
}

/// The low 64 bits of `bi` in two's complement. Integers are 64-bit in this
/// backend, so a constant the MIR folder computed beyond that range (such as
/// `INT64_MIN // -1`) wraps, as the same operation does at run time.
fn wrap_to_i64(bi: &BigInt) -> i64 {
    bi.to_i64()
        .unwrap_or_else(|| (bi & BigInt::from(u64::MAX)).to_u64().unwrap_or(0) as i64)
}

pub fn emit_user_globals(globals: &BTreeSet<String>) -> String {
    let mut out = String::new();
    for g in globals {
        let _ = writeln!(
            out,
            "@__nyk_glob_{} = internal global %NykVal zeroinitializer",
            sanitize_ident(g)
        );
    }
    if !globals.is_empty() {
        out.push('\n');
    }
    out
}

pub fn emit_string_constants(strings: &[(String, String, usize)]) -> String {
    let mut out = String::new();
    let mut ordered: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
    for (sym, escaped, len) in strings {
        ordered.insert(sym.as_str(), (escaped.as_str(), *len));
    }
    for (sym, (escaped, len)) in ordered {
        let _ = writeln!(
            out,
            "{} = private unnamed_addr constant [{} x i8] {}",
            sym, len, escaped
        );
    }
    if !strings.is_empty() {
        out.push('\n');
    }
    out
}
