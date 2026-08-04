//! Consistency verification primitives.
//!
//! The Consistency Checker (in `aion-core`) uses these graph-level primitives
//! to detect decision drift *before* an action executes: has the agent already
//! established a fact that the new action would contradict? If so, the action
//! must be rejected (or rolled back to the last coherent snapshot).

use serde::{Deserialize, Serialize};

use crate::node::NodeId;
use crate::world::WorldModel;

/// Severity of a verification finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    /// Blocking: the action must not execute.
    Error,
    /// Advisory: the action may execute but should be reviewed.
    Warning,
}

/// A single finding produced while verifying an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationIssue {
    /// Severity of the issue.
    pub severity: IssueSeverity,
    /// Stable machine code (e.g. `"CONTRADICTION"`, `"UNKNOWN_TOOL"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Ids of the nodes involved (if any).
    #[serde(default)]
    pub nodes: Vec<NodeId>,
}

/// Result of verifying a proposed action.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// All findings, in detection order.
    pub issues: Vec<VerificationIssue>,
}

impl VerificationReport {
    /// True when the report contains blocking errors.
    pub fn is_blocked(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }

    /// True when the report is clean (no issues at all).
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// Number of blocking errors.
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == IssueSeverity::Error)
            .count()
    }
}

/// Outcome of a verification.
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationOutcome {
    /// The action is coherent with the World Model history.
    Allowed,
    /// The action would contradict the established history.
    Blocked(VerificationReport),
}

/// A proposed action that must be verified against the World Model history.
#[derive(Debug, Clone)]
pub struct ProposedAction {
    /// Action kind (tool name, LLM call type, …).
    pub kind: String,
    /// Free-form arguments.
    pub args: serde_json::Value,
}

/// Verifies a proposed action against the World Model.
///
/// Currently checks the strongest signal available in the graph: whether the
/// action *claims a fact* that contradicts an established observation. The
/// `claim` parameter is an optional JSON pointer path (e.g. `"amount"`) to a
/// fact the action asserts; if the same path already carries a different
/// value, the action is blocked.
pub fn verify_action(
    wm: &WorldModel,
    _action: &ProposedAction,
    claim: Option<(&str, &serde_json::Value)>,
) -> VerificationOutcome {
    let mut report = VerificationReport::default();

    // 1. Existing explicit contradictions in the graph are always surfaced.
    for r in wm.conflicts() {
        report.issues.push(VerificationIssue {
            severity: IssueSeverity::Error,
            code: "EXISTING_CONTRADICTION".into(),
            message: format!(
                "world model already contains a contradiction: {} {} {}",
                r.from.0, r.kind, r.to.0
            ),
            nodes: vec![r.from, r.to],
        });
    }

    // 2. Claimed fact vs. established observations.
    if let Some((path, expected)) = claim {
        for node in wm.nodes_sorted() {
            if let Some(current) = node.data.pointer(&json_pointer(path)) {
                if current != expected {
                    report.issues.push(VerificationIssue {
                        severity: IssueSeverity::Error,
                        code: "FACT_CONTRADICTION".into(),
                        message: format!(
                            "action asserts {}={} but world model records {}={}",
                            path, expected, path, current
                        ),
                        nodes: vec![node.id],
                    });
                }
            }
        }
    }

    if report.is_clean() {
        VerificationOutcome::Allowed
    } else {
        VerificationOutcome::Blocked(report)
    }
}

/// Converts a dot-path into a JSON pointer.
fn json_pointer(path: &str) -> String {
    let mut out = String::from("/");
    out.push_str(&path.replace('.', "/"));
    out
}

/// Helper to resolve a node id to its label (for readable messages).
pub fn label_of(wm: &WorldModel, id: NodeId) -> Option<&str> {
    wm.find_node(id).map(|n| n.label.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RelationType, WorldModel};

    fn wm_with_revenue() -> WorldModel {
        let mut wm = WorldModel::new();
        let e = wm.add_entity("contract");
        let o = wm.add_observation("revenue");
        if let Some(n) = wm.node_mut(o) {
            *n = n
                .clone()
                .with_data(serde_json::json!({"amount": 4200}))
                .unwrap();
        }
        wm.relate(e, o, RelationType::HasProperty).unwrap();
        wm
    }

    #[test]
    fn clean_action_is_allowed() {
        let wm = wm_with_revenue();
        let action = ProposedAction {
            kind: "extract_revenue".into(),
            args: serde_json::json!({}),
        };
        let outcome = verify_action(&wm, &action, Some(("amount", &serde_json::json!(4200))));
        assert_eq!(outcome, VerificationOutcome::Allowed);
    }

    #[test]
    fn contradicting_claim_is_blocked() {
        let wm = wm_with_revenue();
        let action = ProposedAction {
            kind: "extract_revenue".into(),
            args: serde_json::json!({}),
        };
        let outcome = verify_action(&wm, &action, Some(("amount", &serde_json::json!(5000))));
        match outcome {
            VerificationOutcome::Blocked(report) => {
                assert!(report.is_blocked());
                assert_eq!(report.error_count(), 1);
                assert_eq!(report.issues[0].code, "FACT_CONTRADICTION");
            }
            VerificationOutcome::Allowed => panic!("must be blocked"),
        }
    }

    #[test]
    fn existing_contradiction_is_surfaced() {
        let mut wm = wm_with_revenue();
        let e = wm.add_entity("contract2");
        let o = wm.add_observation("revenue2");
        wm.relate(e, o, RelationType::Contradicts).unwrap();
        let action = ProposedAction {
            kind: "noop".into(),
            args: serde_json::json!({}),
        };
        let outcome = verify_action(&wm, &action, None);
        match outcome {
            VerificationOutcome::Blocked(report) => {
                assert!(report
                    .issues
                    .iter()
                    .any(|i| i.code == "EXISTING_CONTRADICTION"));
            }
            VerificationOutcome::Allowed => panic!("must be blocked"),
        }
    }

    #[test]
    fn no_claim_no_conflict_is_allowed() {
        let wm = wm_with_revenue();
        let action = ProposedAction {
            kind: "read_file".into(),
            args: serde_json::json!({"path": "/tmp/x"}),
        };
        let outcome = verify_action(&wm, &action, None);
        assert_eq!(outcome, VerificationOutcome::Allowed);
    }
}
