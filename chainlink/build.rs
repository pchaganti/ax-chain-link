//! Build script to track include_str! dependencies.
//! This ensures cargo rebuilds when template files change.

fn main() {
    // Track .claude files
    println!("cargo:rerun-if-changed=../.claude/settings.json");
    println!("cargo:rerun-if-changed=../.claude/hooks/prompt-guard.py");
    println!("cargo:rerun-if-changed=../.claude/hooks/post-edit-check.py");
    println!("cargo:rerun-if-changed=../.claude/hooks/session-start.py");

    // Track .chainlink/rules files
    println!("cargo:rerun-if-changed=../.chainlink/rules/global.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/project.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/rust.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/python.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/javascript.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/typescript.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/typescript-react.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/javascript-react.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/go.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/java.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/c.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/cpp.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/csharp.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/ruby.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/php.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/swift.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/kotlin.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/scala.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/zig.md");
    println!("cargo:rerun-if-changed=../.chainlink/rules/odin.md");
}
