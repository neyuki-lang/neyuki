// Comprehensive Benchmark Suite for Neyuki Programming Language.
// Benchmarks all core subsystems: Lexer, Parser, Compiler, VM Arithmetic,
// Tables, Strings, Coroutines, Garbage Collector, Crypto, Buffer, and JSON.

#![allow(dead_code)]

use crate::compiler::{compile_source, compile_to_proto};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::vm::machine::VM;
use std::time::{Duration, Instant};

pub struct BenchResult {
    pub name: &'static str,
    pub iterations: usize,
    pub total_duration: Duration,
    pub avg_per_op: Duration,
    pub ops_per_sec: f64,
}

impl BenchResult {
    pub fn display(&self) {
        let avg_us = self.avg_per_op.as_nanos() as f64 / 1_000.0;
        let total_ms = self.total_duration.as_millis();
        println!(
            "{:<32} | {:>8} iters | {:>8.2} ms | {:>10.2} us/op | {:>12.0} ops/s",
            self.name, self.iterations, total_ms as f64, avg_us, self.ops_per_sec
        );
    }
}

fn measure<F: FnMut()>(name: &'static str, iterations: usize, mut f: F) -> BenchResult {
    // Warmup
    f();

    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let total_duration = start.elapsed();
    let avg_per_op = total_duration / (iterations as u32);
    let total_secs = total_duration.as_secs_f64();
    let ops_per_sec = if total_secs > 0.0 {
        iterations as f64 / total_secs
    } else {
        0.0
    };

    BenchResult {
        name,
        iterations,
        total_duration,
        avg_per_op,
        ops_per_sec,
    }
}

fn run_vm_code(code: &str) {
    let stmts = compile_source(code).expect("syntax error in benchmark code");
    let proto = compile_to_proto(&stmts);
    let mut vm = VM::new();
    let _ = vm.execute(proto.clone());
}

// 1. Lexer Benchmark
pub fn bench_lexer() -> BenchResult {
    let source = r#"
        local sum = 0
        for i = 1, 100 do
            local x = i * 2 + 1
            if x % 3 == 0 then
                sum = sum + x
            else
                sum = sum - (x >> 1)
            end
        end
        return sum
    "#;
    measure("1. Lexer Tokenization", 2_000, || {
        let mut lexer = Lexer::new(source);
        let _ = lexer.tokenize();
    })
}

// 2. Parser Benchmark
pub fn bench_parser() -> BenchResult {
    let source = r#"
        function compute(a, b, c)
            local res = {}
            for i = 1, 50 do
                res[i] = (a + b) * c - i
            end
            return res
        end
        return compute(10, 20, 30)
    "#;
    measure("2. AST Parser", 1_000, || {
        let mut parser = Parser::new(source);
        let _ = parser.parse_program();
    })
}

// 3. Compiler & Constant Folding Benchmark
pub fn bench_compiler() -> BenchResult {
    let source = r#"
        local a = 10 * 20 + 30 - 5
        local b = 1 << 4 | 2
        local c = math.sqrt(144) + math.floor(3.14)
        local function calc(x)
            return x * 2 + a + b + c
        end
        return calc(100)
    "#;
    let stmts = compile_source(source).expect("parse error");
    measure("3. Bytecode Compiler & Folding", 1_000, || {
        let _ = compile_to_proto(&stmts);
    })
}

// 4. VM Arithmetic & Loop Benchmark
pub fn bench_arithmetic() -> BenchResult {
    let code = r#"
        local sum = 0
        local i = 0
        while i < 10000 do
            sum = sum + (i * 3 - (i >> 1))
            i = i + 1
        end
        return sum
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("4. VM Arithmetic & Loop", 100, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 5. VM Fibonacci Benchmark
pub fn bench_fibonacci() -> BenchResult {
    let code = r#"
        local a = 0
        local b = 1
        local i = 0
        while i < 1000 do
            local next = a + b
            a = b
            b = next
            i = i + 1
        end
        return b
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("5. VM Fibonacci (1000 iters)", 200, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 6. VM Table Benchmark (Creation, Hash Lookups, Array Insertions)
pub fn bench_tables() -> BenchResult {
    let code = r#"
        local t = {}
        for i = 1, 500 do
            t[i] = i * 2
            t["key_" .. tostring(i)] = i * 3
        end
        local acc = 0
        for i = 1, 500 do
            acc = acc + t[i] + t["key_" .. tostring(i)]
        end
        return acc
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("6. VM Table Operations", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 7. VM String Operations Benchmark (Concatenation, Sub, Formatting)
pub fn bench_strings() -> BenchResult {
    let code = r#"
        local string = require("@neyuki/string")
        local s = "hello, world, neyuki programming language!"
        local out = ""
        for i = 1, 200 do
            local sub = string.sub(s, 1, 12)
            out = string.format("%s-%d", sub, i)
        end
        return out
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("7. VM String Lib Operations", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 8. VM Coroutine Benchmark (Create, Yield, Resume throughput)
pub fn bench_coroutines() -> BenchResult {
    let code = r#"
        local co = require("@neyuki/coroutine")
        local gen = co.create(function()
            local x = 0
            while true do
                x = x + 1
                co.yield(x)
            end
        end)
        local sum = 0
        for i = 1, 1000 do
            local ok, val = co.resume(gen)
            sum = sum + val
        end
        return sum
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("8. VM Coroutines (1000 cycles)", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 9. VM Garbage Collection Benchmark (Cyclic reference collection)
pub fn bench_gc() -> BenchResult {
    let code = r#"
        for i = 1, 100 do
            local a = {}
            local b = {}
            a.b = b
            b.a = a
        end
        collectgarbage("collect")
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("9. VM Cyclic Garbage Collector", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 10. Crypto SHA-256 Hashing Benchmark
pub fn bench_crypto() -> BenchResult {
    let code = r#"
        local crypto = require("@neyuki/crypto")
        local data = "Neyuki high performance cryptographic hashing benchmark payload string 1234567890."
        for i = 1, 100 do
            local h = crypto.hash(data, "sha256")
        end
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("10. VM Crypto SHA-256 (100x)", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 11. Buffer Operations Benchmark (Raw memory read/write & copy)
pub fn bench_buffer() -> BenchResult {
    let code = r#"
        local buffer = require("@neyuki/buffer")
        local b1 = buffer.create(1024)
        local b2 = buffer.create(1024)
        for i = 0, 255 do
            buffer.writeu32(b1, i * 4, i * 100)
        end
        for i = 1, 50 do
            buffer.copy(b2, 0, b1, 0, 1024)
        end
        return buffer.readu32(b2, 0)
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("11. VM Buffer Raw Memory Ops", 100, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// 12. JSON Serialization & Deserialization Benchmark
pub fn bench_json() -> BenchResult {
    let code = r#"
        local json = require("@neyuki/json")
        local record = {
            id = 12345,
            name = "benchmark_test_object",
            active = true,
            scores = {10, 20, 30, 40, 50},
            meta = {
                created_at = "2026-09-17",
                version = "1.0.0"
            }
        }
        for i = 1, 50 do
            local encoded = json.encode(record)
            local decoded = json.decode(encoded)
        end
    "#;
    let stmts = compile_source(code).unwrap();
    let proto = compile_to_proto(&stmts);
    measure("12. VM JSON Encode/Decode", 50, || {
        let mut vm = VM::new();
        let _ = vm.execute(proto.clone());
    })
}

// Run the full benchmark suite and display formatted results
pub fn run_all_benchmarks() {
    println!(
        "========================================================================================="
    );
    println!(
        "                               NEYUKI BENCHMARK SUITE                                    "
    );
    println!(
        "========================================================================================="
    );
    println!(
        "{:<32} | {:>14} | {:>11} | {:>16} | {:>12}",
        "Benchmark Name", "Iterations", "Total Time", "Latency (avg)", "Throughput"
    );
    println!(
        "---------------------------------+----------------+-------------+------------------+-------------"
    );

    let results = [
        bench_lexer(),
        bench_parser(),
        bench_compiler(),
        bench_arithmetic(),
        bench_fibonacci(),
        bench_tables(),
        bench_strings(),
        bench_coroutines(),
        bench_gc(),
        bench_crypto(),
        bench_buffer(),
        bench_json(),
    ];

    for res in &results {
        res.display();
    }

    println!(
        "========================================================================================="
    );
    println!("All benchmarks completed successfully.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmarks_smoke() {
        // Ensure all benchmark functions execute cleanly without panic
        assert!(bench_lexer().ops_per_sec > 0.0);
        assert!(bench_parser().ops_per_sec > 0.0);
        assert!(bench_compiler().ops_per_sec > 0.0);
        assert!(bench_arithmetic().ops_per_sec > 0.0);
        assert!(bench_fibonacci().ops_per_sec > 0.0);
        assert!(bench_tables().ops_per_sec > 0.0);
        assert!(bench_strings().ops_per_sec > 0.0);
        assert!(bench_coroutines().ops_per_sec > 0.0);
        assert!(bench_gc().ops_per_sec > 0.0);
        assert!(bench_crypto().ops_per_sec > 0.0);
        assert!(bench_buffer().ops_per_sec > 0.0);
        assert!(bench_json().ops_per_sec > 0.0);
    }
}
