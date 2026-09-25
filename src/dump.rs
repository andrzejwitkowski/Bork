//! ASCII arena dump for `--dump-arenas`.

use crate::sema::{ArenaNode, ArenaReport, BindingInfo, Ownership};

pub fn dump_arenas(report: &ArenaReport) -> String {
    let mut out = String::from("Arenas\n");
    let n = report.roots.len();
    for (i, root) in report.roots.iter().enumerate() {
        dump_node(&mut out, root, "", i + 1 == n);
    }
    out
}

fn dump_lines(node: &ArenaNode) -> Vec<&BindingInfo> {
    let mut lines: Vec<_> = node.bindings.iter().collect();
    lines.extend(
        node.observations
            .iter()
            .filter(|b| !matches!(b.ownership, Ownership::Local)),
    );
    lines
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
    let lines = dump_lines(node);
    let total = lines.len() + node.children.len();
    let mut idx = 0;

    for b in lines {
        idx += 1;
        let last = idx == total;
        let br = if last { "└── " } else { "├── " };
        out.push_str(&child_prefix);
        out.push_str(br);
        out.push_str(&b.name);
        out.push(' ');
        out.push_str(&b.ownership.dump_tag());
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
    use crate::frontend::check;

    #[test]
    fn dump_contains_fun_and_for() {
        let result = check(crate::MVP_SAMPLE);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let report = result.report.unwrap();
        let text = dump_arenas(&report);
        assert!(text.contains("fun main"), "{text}");
        assert!(text.contains("ForLoop"), "{text}");
        assert!(text.contains("Arenas\n"), "{text}");
    }
}
