# Bork  Bork 

> *A high-performance, native systems programming language with Kotlin-like expressive syntax, region-based memory management, and zero GC. When your code is invalid... the compiler borks.*

---

## 🧅 Why Bork?

Bork is designed to bridge the gap between high-level developer ergonomics (clean syntax, trailing closures, expressive DSL structures) and bare-metal systems performance (zero Garbage Collector, deterministic cleanup, LLVM code generation). 

Instead of fighting complex, single-variable borrow checkers like Rust's lifetime system, Bork relies on **hierarchical memory regions (arenas)** bound directly to code blocks (`{}`). 

### Core Pillars
1. **Kotlin-Like Ergonomics:** Clean syntax, block-based structure, first-class functions, and seamless DSL-friendly trailing closures.
2. **Region-Based Memory Management:** Curly braces `{}` define memory regions. Everything allocated within a block is cleaned up in bulk when the block closes. Zero manual `free`, zero runtime GC pauses.
3. **Loop Region Resetting:** Loop iterations automatically reset their arena offset, ensuring $O(1)$ memory growth inside heavy loops.
4. **Move & Copy Semantics:** Safe cross-region data flow without pointer invalidation.
5. **Native Performance:** Compiles directly to machine code via **LLVM**, providing C/C++ level execution speeds.

---

## 🚀 Language Syntax Preview

```kotlin
// A function taking types and a closure (lambda) as its last parameter
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main() {
    // Region: main block
    {
        var accumulator = 0
        val threshold = 5
        
        // Loop block acts as a repeating sub-region
        for (i in 0..10) {
            // DSL-style closure execution (trailing lambda)
            accumulator = action(accumulator, i) { acc, current ->
                if (current > threshold) {
                    acc + current
                } else {
                    acc
                }
            }
        }
    } 
    // End of main block -> entire arena memory released instantly in bulk.
}
```

---

## Editor / LSP

Parse diagnostics are available through a stdio language server:

```bash
cargo build --bin bork-lsp
# Point your editor's LSP client at: target/debug/bork-lsp
# Language id: bork
```

On document open/change the server runs `bork::parse` and publishes errors as squiggles. Hover, completion, and go-to-definition are not implemented yet.
