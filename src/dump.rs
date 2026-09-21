//! ASCII arena dump for `--dump-arenas`.

use crate::sema::{ArenaNode, ArenaReport, Ownership};

pub fn dump_arenas(report: &ArenaReport) -> String {
    let mut out = String::from("Arenas\n");
    let n = report.roots.len();
    for (i, root) in report.roots.iter().enumerate() {
        dump_node(&mut out, root, "", i + 1 == n);
    }
    out
}

fn dump_node(out: &mut String, node: &ArenaNode, prefix: &str, is_last: bool) {
    let branch = if is_last { "└── " } else { "├── " };
    let mut label = node.label.clone();
    if node.compacted_braces > 0 {
        label.push_str(&format!(
            " (compacted {} brace{})",
            node.compacted_braces,
            if node.compacted_braces == 1 { "" } else { "s" }
        ));
    }
    out.push_str(prefix);
    out.push_str(branch);
    out.push_str(&label);
    out.push('\n');

    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
    let total = node.bindings.len() + node.children.len();
    let mut idx = 0;

    for b in &node.bindings {
        idx += 1;
        let last = idx == total;
        let br = if last { "└── " } else { "├── " };
        let own = match &b.ownership {
            Ownership::Local => "[Local]".to_string(),
            Ownership::Copy => "[Copy]".to_string(),
            Ownership::Shared { from } => format!("[Shared ← {from}]"),
            Ownership::Moved { from } => format!("[Moved ← {from}]"),
        };
        out.push_str(&child_prefix);
        out.push_str(br);
        out.push_str(&b.name);
        out.push(' ');
        out.push_str(&own);
        out.push('\n');
    }

    for child in &node.children {
        idx += 1;
        dump_node(out, child, &child_prefix, idx == total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, sema::analyze};

    #[test]
    fn dump_contains_fun_and_for() {
        let prog = parse(crate::MVP_SAMPLE).unwrap();
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty());
        let text = dump_arenas(&report);
        assert!(text.contains("fun main"), "{text}");
        assert!(text.contains("ForLoop"), "{text}");
        assert!(text.contains("Arenas\n"), "{text}");
    }
}
