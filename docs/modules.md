# Neyuki Module System Specification

This document details the architecture, resolution algorithm, caching semantics, and export conventions of the Neyuki module system.

## 1. Module Definition and Execution

A module in Neyuki is a self-contained `.nyk` source file or precompiled `.nykb` bytecode artifact.

### 1.1 Scope and Isolation
- Each module executes within its own private top-level chunk.
- Local variables (`local`, `const`) defined in a module are private to that module chunk and are never accessible from importing files.
- The return value of the module chunk (via `return <value>`) is the exported interface of the module.
- If a module terminates without an explicit return statement, it evaluates to `nil`.

### 1.2 Export Pattern
The standard convention is to export a table containing functions, constants, or types:

```lua
local math_utils = {}

const function math_utils.add(a: int, b: int): int
    return a + b
end

const function math_utils.clamp(val: number, min: number, max: number): number
    if val < min then return min end
    if val > max then return max end
    return val
end

return math_utils
```

---

## 2. Module Resolution (`require`)

Modules are loaded at runtime using the global `require(module_path)` function.

### 2.1 Resolution Hierarchy
When `require(path)` is invoked, the runtime resolves the target in the following order:

1. **Module Cache Check**:
   If the canonical name has already been resolved and executed, the cached exported value is returned immediately without re-executing the module code.

2. **Standard and Bundled Libraries (`@neyuki/`)**:
   Paths matching `@neyuki/<name>` or `<name>` (where `<name>` is one of the built-in libraries: `bit`, `bit32`, `buffer`, `coroutine`, `crypto`, `debug`, `fs`, `hid`, `http`, `io`, `json`, `math`, `os`, `sql`, `string`, `table`, `ui`, `utf8`) are served directly from the embedded standard library distribution.

3. **Package Manager Dependencies (`@pkg/`)**:
   Paths starting with `@pkg/<package_name>` resolve against packages installed in the local `neyuki_packages/` directory or global cache configured via `NEYUKI_PATH`.

4. **Relative and Filesystem Paths**:
   Paths ending in `.nyk` or `.nykb`, or relative module identifiers (e.g., `./utils`, `components/parser`) resolve against the directory of the currently executing script.

### 2.2 Security and Path Traversal Restrictions
To ensure sandboxing integrity and prevent arbitrary filesystem escape, `require` rejects:
- Null bytes (`\0`).
- Relative parent path traversal (`..`).
- Absolute root paths (`/` or Windows drive prefixes `C:\`).

Path traversal attempts trigger an immediate security exception:
```text
runtime error[E0200]: security error: path traversal forbidden in require: '../sensitive'
```

---

## 3. Caching and Lifecycle

### 3.1 Idempotent Execution
Modules execute once per VM instance. The returned value is cached in `vm.modules[canonical_name]`. Subsequent calls to `require` with the same module specifier receive the cached instance.

### 3.2 Circular Dependencies
If module `A` requires module `B`, and module `B` requires module `A`:
1. While `A` is executing, its entry in `vm.modules` is unresolved.
2. If `B` immediately accesses fields on `A` during top-level load, `nil` is observed.
3. Modules intended for mutual recursion must defer cross-module function calls until execution time inside function bodies rather than during top-level module initialization.

### 3.3 Dynamic Reloading
In standard execution, modules remain cached for the lifetime of the `VM` instance. Development environments (such as the REPL or interactive runner) can invalidate cache keys directly via `package.loaded[name] = nil`.
