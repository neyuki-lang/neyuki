/// Sentinel for `argc`, `retc` and `Return`'s `count` meaning "however many
/// values are actually there", i.e. up to the VM's current stack top. Register
/// indices never reach 255, so it cannot be mistaken for a real count.
pub const MULTRET: u8 = u8::MAX;

#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    // Load nil into register
    LoadNil { dst: u8 },
    // Load boolean into register
    LoadBool { dst: u8, val: bool },
    // Load integer immediate into register
    LoadInt { dst: u8, val: i32 },
    // Load constant from constant pool into register
    LoadK { dst: u8, k: u16 },
    // Move value between registers
    Move { dst: u8, src: u8 },

    // Global variable access
    GetGlobal { dst: u8, name_k: u16 },
    SetGlobal { src: u8, name_k: u16 },

    // Upvalue access for closures
    GetUpval { dst: u8, upval_idx: u8 },
    SetUpval { src: u8, upval_idx: u8 },

    // Table operations
    NewTable { dst: u8 },
    GetTable { dst: u8, table: u8, key: u8 },
    SetTable { table: u8, key: u8, val: u8 },
    GetTableK { dst: u8, table: u8, key_k: u16 },
    SetTableK { table: u8, key_k: u16, val: u8 },
    AppendArray { table: u8, src: u8 },

    // Binary arithmetic operators: dst = a OP b
    Add { dst: u8, a: u8, b: u8 },
    Sub { dst: u8, a: u8, b: u8 },
    Mul { dst: u8, a: u8, b: u8 },
    Div { dst: u8, a: u8, b: u8 },
    IDiv { dst: u8, a: u8, b: u8 },
    Mod { dst: u8, a: u8, b: u8 },
    Pow { dst: u8, a: u8, b: u8 },

    // Bitwise operators: dst = a OP b
    BitAnd { dst: u8, a: u8, b: u8 },
    BitOr { dst: u8, a: u8, b: u8 },
    BitXor { dst: u8, a: u8, b: u8 },
    Shl { dst: u8, a: u8, b: u8 },
    Shr { dst: u8, a: u8, b: u8 },
    LShl { dst: u8, a: u8, b: u8 },
    LShr { dst: u8, a: u8, b: u8 },

    // String concatenation: dst = a .. b
    Concat { dst: u8, a: u8, b: u8 },

    // Unary operators: dst = OP src
    Unm { dst: u8, src: u8 },
    Not { dst: u8, src: u8 },
    Len { dst: u8, src: u8 },
    BitNot { dst: u8, src: u8 },

    // Nil coalescing: dst = a ?? b
    Coalesce { dst: u8, a: u8, b: u8 },

    // Comparisons with conditional jumps
    Eq { a: u8, b: u8, jump_if_false: i16 },
    Ne { a: u8, b: u8, jump_if_false: i16 },
    Lt { a: u8, b: u8, jump_if_false: i16 },
    Le { a: u8, b: u8, jump_if_false: i16 },
    Gt { a: u8, b: u8, jump_if_false: i16 },
    Ge { a: u8, b: u8, jump_if_false: i16 },

    // Test register truthiness and jump if false
    Test { reg: u8, jump_if_false: i16 },
    // Unconditional relative jump
    Jump { offset: i16 },

    // Function calls and returns. `argc`/`retc` may be MULTRET to pass on, or
    // keep, every value available rather than a fixed number.
    Call { callee: u8, argc: u8, retc: u8 },
    Return { base: u8, count: u8 },
    // Instantiate closure for nested proto
    Closure { dst: u8, proto_idx: u16 },
    // Expand varargs into registers
    Vararg { dst: u8, count: u8 },

    // Numeric for loop preparation: R(base) -= R(base+2); jump to ForLoop
    ForPrep { base: u8, jump: i16 },
    // Numeric for loop step and test: R(base) += R(base+2); if valid, R(base+3) = R(base) and jump
    ForLoop { base: u8, jump: i16 },
    // Batch array initialization: append `count` registers starting at `base` to
    // `table`; MULTRET appends everything up to the stack top
    SetList { table: u8, base: u8, count: u8 },
    // Generic for loop: call R(base)(R(base+1), R(base+2)), store retc results at R(base+3)..
    TForCall { base: u8, retc: u8 },
    // Generic for loop test: if R(base+3)==nil jump forward; else R(base+2) = R(base+3)
    TForLoop { base: u8, jump: i16 },
}
