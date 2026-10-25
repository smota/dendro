//! Walks explicit `session_edges` rows to a root caller.
//! A cycle stops that branch and is flagged. An empty parent is unresolved.
//! No root id is invented.

use std::collections::{BTreeMap, BTreeSet};

/// Caller→child link already stored. `parent_session` empty means the source named no parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLink {
    pub parent_session: String,
    pub child_session: String,
    pub relation: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageRoots {
    pub roots: BTreeSet<String>,
    pub cycle: bool,
    pub unresolved: bool,
}

/// Direct children of `session`. Status does not drop a child that finished later.
pub fn children_of<'a>(edges: &'a [SessionLink], session: &str) -> Vec<&'a str> {
    let mut kids: Vec<&str> = edges
        .iter()
        .filter(|edge| edge.parent_session == session && !edge.child_session.is_empty())
        .map(|edge| edge.child_session.as_str())
        .collect();
    kids.sort_unstable();
    kids.dedup();
    kids
}

/// Parent walk from `session`. A session with no parent edge is its own root.
/// Empty parent ids flag `unresolved` and are not turned into a placeholder root.
pub fn lineage_roots(edges: &[SessionLink], session: &str) -> LineageRoots {
    let mut by_child: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges {
        if edge.child_session.is_empty() {
            continue;
        }
        let parents = by_child.entry(edge.child_session.as_str()).or_default();
        if !parents.contains(&edge.parent_session.as_str()) {
            parents.push(edge.parent_session.as_str());
        }
    }
    let mut roots = BTreeSet::new();
    let mut cycle = false;
    let mut unresolved = false;
    let mut stack = Vec::new();
    let mut seen = BTreeSet::new();
    walk(
        session,
        &by_child,
        &mut stack,
        &mut seen,
        &mut roots,
        &mut cycle,
        &mut unresolved,
    );
    LineageRoots {
        roots,
        cycle,
        unresolved,
    }
}

fn walk<'a>(
    node: &'a str,
    by_child: &BTreeMap<&'a str, Vec<&'a str>>,
    stack: &mut Vec<&'a str>,
    seen: &mut BTreeSet<&'a str>,
    roots: &mut BTreeSet<String>,
    cycle: &mut bool,
    unresolved: &mut bool,
) {
    if stack.contains(&node) {
        *cycle = true;
        return;
    }
    if !seen.insert(node) {
        return;
    }
    let Some(parents) = by_child.get(node) else {
        roots.insert(node.to_string());
        return;
    };
    stack.push(node);
    let mut followed = false;
    for parent in parents {
        if parent.is_empty() {
            *unresolved = true;
            continue;
        }
        followed = true;
        walk(parent, by_child, stack, seen, roots, cycle, unresolved);
    }
    stack.pop();
    if !followed {
        *unresolved = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{children_of, lineage_roots, SessionLink};
    use std::collections::BTreeSet;

    fn link(parent: &str, child: &str, status: &str) -> SessionLink {
        SessionLink {
            parent_session: parent.into(),
            child_session: child.into(),
            relation: "spawn".into(),
            status: status.into(),
        }
    }

    #[test]
    fn chain_resolves_to_the_root_caller() {
        let edges = [link("a", "b", "started"), link("b", "c", "started")];
        let got = lineage_roots(&edges, "c");
        assert_eq!(got.roots, BTreeSet::from(["a".into()]));
        assert!(!got.cycle);
        assert!(!got.unresolved);
        assert_eq!(children_of(&edges, "b"), vec!["c"]);
    }

    #[test]
    fn diamond_keeps_both_parents() {
        let edges = [link("a", "c", "started"), link("b", "c", "started")];
        let got = lineage_roots(&edges, "c");
        assert_eq!(got.roots, BTreeSet::from(["a".into(), "b".into()]));
        assert!(!got.cycle);
        assert!(!got.unresolved);
        assert_eq!(children_of(&edges, "a"), vec!["c"]);
        assert_eq!(children_of(&edges, "b"), vec!["c"]);
    }

    #[test]
    fn cycle_stops_and_invents_no_root() {
        let edges = [link("a", "b", "started"), link("b", "a", "started")];
        let got = lineage_roots(&edges, "a");
        assert!(got.roots.is_empty());
        assert!(got.cycle);
        assert!(!got.unresolved);
    }

    #[test]
    fn orphan_missing_parent_is_unresolved() {
        let edges = [link("", "orphan", "started")];
        let got = lineage_roots(&edges, "orphan");
        assert!(got.roots.is_empty());
        assert!(!got.cycle);
        assert!(got.unresolved);
        let alone = lineage_roots(&[], "solo");
        assert_eq!(alone.roots, BTreeSet::from(["solo".into()]));
        assert!(!alone.unresolved);
    }

    #[test]
    fn finished_child_still_links_after_its_parent() {
        let edges = [link("parent", "child", "finished")];
        let got = lineage_roots(&edges, "child");
        assert_eq!(got.roots, BTreeSet::from(["parent".into()]));
        assert!(!got.cycle);
        assert!(!got.unresolved);
        assert_eq!(children_of(&edges, "parent"), vec!["child"]);
    }
}
