//! Random object graphs for the garbage collector, and buffer accesses at
//! hostile offsets.
//!
//! An input is a stream of three-byte operations (op, a, b). `INTERPRETER`
//! replays them to build tables with cycles, weak tables and finalizers that
//! resurrect objects, to drop and root them, and to collect. After each
//! collection it checks that every rooted table, and every object a
//! finalizer resurrected, still has its fields; a missing one is reported as
//! a crash. The buffer operations also run straight against `VmBuffer`.

use std::cell::RefCell;

use rand::Rng;
use rand::rngs::StdRng;

use super::super::corpus::{Corpus, INTERESTING_U64, sandbox_vm};
use super::super::driver::Target;
use crate::bytecode::proto::Proto;
use crate::compiler::{Options, compile_program};
use crate::vm::buffer::VmBuffer;
use crate::vm::value::Value;

pub const TARGET: Target = Target {
    name: "memory",
    extension: "bin",
    default_seconds: 30,
    regressions,
    generate,
    run,
};

fn regressions() {
    use super::mem_fuzz::*;
    fuzz_buffer_integer_overflow();
    fuzz_buffer_copy_boundaries();
    fuzz_gc_cyclic_stress();
    fuzz_vm_recursion_protection();
}

fn generate(_corpus: &Corpus, rng: &mut StdRng) -> Vec<u8> {
    let n = rng.gen_range(10..400);
    let mut bytes = Vec::with_capacity(n * 3);
    for _ in 0..n {
        // Mostly node building and linking, so graphs get big enough.
        let op = if rng.gen_bool(0.5) {
            rng.gen_range(0..4)
        } else {
            rng.r#gen()
        };
        bytes.extend_from_slice(&[op, rng.r#gen(), rng.r#gen()]);
    }
    bytes
}

/// Replays `ops`, a hex string of (op, a, b) byte triples.
const INTERPRETER: &str = "
local nodes, roots, resurrected, weak, bufs = {}, {}, {}, {}, {}
local made = 0
local modes = {\"k\", \"v\", \"kv\"}
local function node(i)
    if #nodes == 0 then return nil end
    return nodes[i % #nodes + 1]
end
local function check()
    for _, r in roots do
        if r[1].id != r[2] then error(\"FUZZ-ORACLE: a rooted table lost its fields\") end
    end
    for _, o in resurrected do
        if o.id == nil then error(\"FUZZ-ORACLE: a resurrected table lost its fields\") end
    end
end
local function finalize(o)
    if #resurrected < 64 then resurrected[#resurrected + 1] = o end
end
for p = 1, #ops - 5, 6 do
    local op = tonumber(string.sub(ops, p, p + 1), 16) % 14
    local a = tonumber(string.sub(ops, p + 2, p + 3), 16)
    local b = tonumber(string.sub(ops, p + 4, p + 5), 16)
    if op <= 1 then
        made += 1
        nodes[#nodes + 1] = {id = made}
    elseif op == 2 then
        local x = node(a)
        if x then x.e = node(b) end
    elseif op == 3 then
        local x = node(a)
        if x then x[node(b) ?? 1] = node(a + 1) end
    elseif op == 4 then
        nodes[a % (#nodes + 1) + 1] = nil
    elseif op == 5 then
        local x = node(a)
        if x then roots[#roots + 1] = {x, x.id} end
    elseif op == 6 then
        local w = setmetatable({}, {__mode = modes[b % 3 + 1]})
        w[1] = node(a)
        w[node(b) ?? 2] = node(a)
        weak[#weak + 1] = w
    elseif op == 7 then
        local x = node(a)
        if x and getmetatable(x) == nil then setmetatable(x, {__gc = finalize}) end
    elseif op == 8 then
        collectgarbage(\"collect\")
        check()
    elseif op == 9 then
        collectgarbage(\"step\")
        check()
    elseif op == 10 then
        nodes = {node(a), node(b)}
    elseif op == 11 then
        bufs[#bufs + 1] = buffer.create(a % 64)
    elseif op == 12 and #bufs > 0 then
        pcall(buffer.copy, bufs[a % #bufs + 1], a - 32, bufs[b % #bufs + 1], b - 32, a % 16)
    elseif op == 13 and #bufs > 0 then
        local buf = bufs[a % #bufs + 1]
        pcall(buffer.writeu32, buf, b - 128, a)
        pcall(buffer.readbits, buf, b - 64, a % 40)
        pcall(buffer.fill, buf, a - 16, b, b % 32)
    end
end
collectgarbage(\"collect\")
check()
";

thread_local! {
    static COMPILED: RefCell<Option<Proto>> = const { RefCell::new(None) };
}

fn run(input: &[u8]) {
    let mut buffer = VmBuffer::new(16);
    for op in input.chunks(3) {
        if let [_, a, b] = *op {
            let offset = INTERESTING_U64[a as usize % INTERESTING_U64.len()] as usize;
            let offset = offset.wrapping_add(b as usize).wrapping_sub(128);
            let _ = buffer.read_u8(offset);
            let _ = buffer.write_u8(offset, b);
            let _ = buffer.range(offset, b as usize);
        }
    }

    let proto = COMPILED.with(|compiled| {
        compiled
            .borrow_mut()
            .get_or_insert_with(|| {
                compile_program(INTERPRETER, "fuzz-memory", Options::UNCHECKED)
                    .expect("the memory fuzz interpreter compiles")
            })
            .clone()
    });
    let hex: String = input.iter().map(|b| format!("{:02x}", b)).collect();
    let mut vm = sandbox_vm();
    vm.set_global("ops", Value::string(hex));
    if let Err(err) = vm.execute(proto)
        && err.contains("FUZZ-ORACLE")
    {
        panic!("{}", err);
    }
}
