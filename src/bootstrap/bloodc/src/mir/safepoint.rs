//! Safepoint insertion pass (INFRA-02 Phase 4).
//!
//! Inserts `Safepoint` statements at loop back-edge targets (loop headers)
//! to enable cooperative preemption. At runtime, each safepoint checks a
//! per-thread preemption flag and yields if set (~1 cycle when not preempting).
//!
//! Insertion points:
//! - Loop headers (targets of back-edges in the CFG)
//! - Function prologues (entry block) — optional, controlled by config
//!
//! Back-edges are detected using reverse postorder: an edge (A → B) is a
//! back-edge if B has a lower RPO index than A (i.e., B is visited before A).

use std::collections::HashSet;

use super::body::MirBody;
use super::types::{BasicBlockId, Statement, StatementKind};
use crate::span::Span;

/// Insert safepoints into a MIR body.
///
/// Inserts `Safepoint` statements at:
/// 1. The start of each loop header block (target of a back-edge)
/// 2. Optionally, the function prologue (entry block)
///
/// Returns the number of safepoints inserted.
pub fn insert_safepoints(body: &mut MirBody, insert_prologue: bool) -> usize {
    let rpo = body.reverse_postorder();

    // Build RPO index map: block → position in RPO
    let mut rpo_index = vec![0usize; body.basic_blocks.len()];
    for (i, &bb) in rpo.iter().enumerate() {
        rpo_index[bb.index()] = i;
    }

    // Find loop headers: targets of back-edges
    // A back-edge is (A → B) where rpo_index[A] >= rpo_index[B]
    let mut loop_headers: HashSet<BasicBlockId> = HashSet::new();
    for &bb in &rpo {
        if let Some(block) = body.get_block(bb) {
            for succ in block.successors() {
                if succ.index() < rpo_index.len()
                    && rpo_index[succ.index()] <= rpo_index[bb.index()]
                {
                    loop_headers.insert(succ);
                }
            }
        }
    }

    let mut count = 0;

    // Insert safepoint at the beginning of each loop header
    for &header in &loop_headers {
        if let Some(block) = body.basic_blocks.get_mut(header.index()) {
            let safepoint = Statement {
                kind: StatementKind::Safepoint,
                span: Span::dummy(),
            };
            block.statements.insert(0, safepoint);
            count += 1;
        }
    }

    // Optionally insert safepoint at function prologue
    if insert_prologue {
        if let Some(entry) = body.basic_blocks.get_mut(BasicBlockId::ENTRY.index()) {
            // Don't double-insert if entry is already a loop header
            if !loop_headers.contains(&BasicBlockId::ENTRY) {
                let safepoint = Statement {
                    kind: StatementKind::Safepoint,
                    span: Span::dummy(),
                };
                entry.statements.insert(0, safepoint);
                count += 1;
            }
        }
    }

    count
}
