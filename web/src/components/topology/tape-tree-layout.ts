/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/**
 * Pure layout helpers for the tape lineage view (task #7 of #1999).
 *
 * The reducer folds `tape_forked` topology events into a forest of tape
 * nodes — one tree per root tape per session. The layered layout assigns
 * each node an `(x, y)` based on its `depth` (column) and the in-order
 * traversal index of its subtree (row), the same shape `dot` would emit
 * for a small DAG. We avoid pulling in d3 / dagre because the data
 * volume is tiny (≤ a few dozen nodes per session) and the layout is
 * deterministic, which keeps the SVG snapshot-testable.
 */

import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

/** A tape node in the lineage forest. */
export interface TapeNode {
  /** Stable id — `${sessionKey}::${tapeName}`. */
  id: string;
  /** Session that owns the tape. Forks never cross sessions. */
  sessionKey: string;
  /** Tape name as known to `TapeStore` (e.g. `main` or `fork-…`). */
  tapeName: string;
  /** Depth from the tree root (root = 0). */
  depth: number;
  /** Anchor on which this tape was forked, if any. Root tapes have no anchor. */
  forkedAtAnchor: string | null;
  /** Wall-clock-ish ordering: the `seq` of the `tape_forked` event that created
   *  this node. Roots use 0 so they sort first. */
  createdAtSeq: number;
}

/** A directed edge `parent → child` in the lineage forest. */
export interface TapeEdge {
  parentId: string;
  childId: string;
  /** Anchor on which the child was forked from the parent. */
  forkedAtAnchor: string | null;
}

/** A node placed in the SVG coordinate system by {@link layoutTapeForest}. */
export interface PositionedTapeNode extends TapeNode {
  /** Column = depth × COL_WIDTH + padding. */
  x: number;
  /** Row = traversal index × ROW_HEIGHT + padding. */
  y: number;
}

/** Result of the layout pass — ready to render as SVG. */
export interface TapeForestLayout {
  nodes: PositionedTapeNode[];
  edges: TapeEdge[];
  width: number;
  height: number;
}

// ---------------------------------------------------------------------------
// Layout constants — mechanism, not config. The view is a tiny diagnostic
// aid (≤ a few dozen nodes); a deploy operator has no reason to retune
// these. See `docs/guides/anti-patterns.md` ("mechanism constants are
// not config").
// ---------------------------------------------------------------------------

export const NODE_WIDTH = 168;
export const NODE_HEIGHT = 32;
export const COL_GAP = 56;
export const ROW_GAP = 12;
export const PADDING = 12;

const COL_STRIDE = NODE_WIDTH + COL_GAP;
const ROW_STRIDE = NODE_HEIGHT + ROW_GAP;

/**
 * Fold a `TopologyEventEntry[]` buffer into a forest of tape lineage
 * trees. Pure — safe to call inside `useMemo`.
 *
 * Returns one logical forest spanning every session in the buffer; the
 * caller groups nodes by `sessionKey` if it wants per-session subtrees.
 *
 * Root tapes (tapes that appear only as `forked_from` and never as
 * `child_tape`) are synthesised so the tree has a real root to anchor
 * the layout. Their `forkedAtAnchor` is always `null`.
 */
export function buildTapeForest(events: TopologyEventEntry[]): {
  nodes: TapeNode[];
  edges: TapeEdge[];
} {
  const nodesById = new Map<string, TapeNode>();
  const edges: TapeEdge[] = [];
  const seenEdge = new Set<string>();

  const idOf = (sessionKey: string, tapeName: string): string => `${sessionKey}::${tapeName}`;

  for (const entry of events) {
    const frame = entry.event;
    if (frame.type !== 'tape_forked') continue;

    const parentId = idOf(frame.parent_session, frame.forked_from);
    const childId = idOf(frame.parent_session, frame.child_tape);

    // Seed the parent as a root candidate. If we later see it as a
    // `child_tape` of some other fork, `assignDepths` will reparent it.
    if (!nodesById.has(parentId)) {
      nodesById.set(parentId, {
        id: parentId,
        sessionKey: frame.parent_session,
        tapeName: frame.forked_from,
        depth: 0,
        forkedAtAnchor: null,
        createdAtSeq: 0,
      });
    }

    const existingChild = nodesById.get(childId);
    if (existingChild) {
      // Duplicate `tape_forked` for the same child — keep the earliest
      // (lowest seq) anchor. In practice this should not happen because
      // tape names are allocated uniquely by `TapeStore::fork`, but the
      // reducer must be idempotent against socket replays.
      if (entry.seq < existingChild.createdAtSeq || existingChild.createdAtSeq === 0) {
        existingChild.forkedAtAnchor = frame.forked_at_anchor ?? null;
        existingChild.createdAtSeq = entry.seq;
      }
    } else {
      nodesById.set(childId, {
        id: childId,
        sessionKey: frame.parent_session,
        tapeName: frame.child_tape,
        depth: 0,
        forkedAtAnchor: frame.forked_at_anchor ?? null,
        createdAtSeq: entry.seq,
      });
    }

    const edgeKey = `${parentId}→${childId}`;
    if (!seenEdge.has(edgeKey)) {
      seenEdge.add(edgeKey);
      edges.push({
        parentId,
        childId,
        forkedAtAnchor: frame.forked_at_anchor ?? null,
      });
    }
  }

  assignDepths(nodesById, edges);

  return { nodes: [...nodesById.values()], edges };
}

/**
 * BFS from each root node (a node with no incoming edge) to fill in
 * `depth`. Cycles are impossible by construction — `tape_forked` is
 * append-only and `child_tape` is freshly allocated — but we still cap
 * traversal depth defensively.
 */
function assignDepths(nodesById: Map<string, TapeNode>, edges: TapeEdge[]): void {
  const childrenOf = new Map<string, string[]>();
  const incoming = new Map<string, number>();
  for (const node of nodesById.values()) incoming.set(node.id, 0);
  for (const edge of edges) {
    const list = childrenOf.get(edge.parentId) ?? [];
    list.push(edge.childId);
    childrenOf.set(edge.parentId, list);
    incoming.set(edge.childId, (incoming.get(edge.childId) ?? 0) + 1);
  }

  const queue: string[] = [];
  for (const [id, count] of incoming) {
    if (count === 0) {
      const node = nodesById.get(id);
      if (node) {
        node.depth = 0;
        queue.push(id);
      }
    }
  }

  // Defensive cap — a tape forest grows at most one node per
  // `tape_forked` event so a cycle would mean malformed input.
  const cap = nodesById.size + 1;
  let visited = 0;
  while (queue.length > 0 && visited < cap) {
    visited += 1;
    const id = queue.shift() as string;
    const parent = nodesById.get(id);
    if (!parent) continue;
    for (const childId of childrenOf.get(id) ?? []) {
      const child = nodesById.get(childId);
      if (!child) continue;
      const candidate = parent.depth + 1;
      if (candidate > child.depth) {
        child.depth = candidate;
        queue.push(childId);
      }
    }
  }
}

/**
 * Place every node on a 2D grid by depth (x) and per-session traversal
 * order (y). Per-session because forks never cross sessions, so each
 * session's subtree is laid out as its own block stacked vertically.
 *
 * The traversal order is a stable DFS keyed on `createdAtSeq` so the
 * earliest fork appears at the top of its subtree — matches the user's
 * mental model of reading the topology stream top-to-bottom.
 */
export function layoutTapeForest(forest: {
  nodes: TapeNode[];
  edges: TapeEdge[];
}): TapeForestLayout {
  const { nodes, edges } = forest;
  if (nodes.length === 0) {
    return { nodes: [], edges: [], width: 0, height: 0 };
  }

  const childrenOf = new Map<string, string[]>();
  const incoming = new Map<string, number>();
  for (const node of nodes) incoming.set(node.id, 0);
  for (const edge of edges) {
    const list = childrenOf.get(edge.parentId) ?? [];
    list.push(edge.childId);
    childrenOf.set(edge.parentId, list);
    incoming.set(edge.childId, (incoming.get(edge.childId) ?? 0) + 1);
  }

  // Sort children deterministically — earliest-forked first, then by
  // tape name as a stable tiebreaker.
  const nodeById = new Map(nodes.map((n) => [n.id, n] as const));
  for (const list of childrenOf.values()) {
    list.sort((a, b) => {
      const na = nodeById.get(a);
      const nb = nodeById.get(b);
      if (!na || !nb) return 0;
      if (na.createdAtSeq !== nb.createdAtSeq) return na.createdAtSeq - nb.createdAtSeq;
      return na.tapeName.localeCompare(nb.tapeName);
    });
  }

  // Group roots by session so each session's subtree is contiguous in
  // the row axis. Sessions sort by the seq of their first fork event so
  // the root session (which forks first) stays at the top.
  const rootsBySession = new Map<string, TapeNode[]>();
  for (const node of nodes) {
    if ((incoming.get(node.id) ?? 0) > 0) continue;
    const list = rootsBySession.get(node.sessionKey) ?? [];
    list.push(node);
    rootsBySession.set(node.sessionKey, list);
  }
  const sessionsOrdered = [...rootsBySession.entries()].sort(([, a], [, b]) => {
    const sa = Math.min(...a.map((n) => firstForkSeqUnder(n.id, childrenOf, nodeById)));
    const sb = Math.min(...b.map((n) => firstForkSeqUnder(n.id, childrenOf, nodeById)));
    return sa - sb;
  });

  const positioned: PositionedTapeNode[] = [];
  let row = 0;
  let maxDepth = 0;

  const visit = (id: string): void => {
    const node = nodeById.get(id);
    if (!node) return;
    if (node.depth > maxDepth) maxDepth = node.depth;
    positioned.push({
      ...node,
      x: PADDING + node.depth * COL_STRIDE,
      y: PADDING + row * ROW_STRIDE,
    });
    row += 1;
    for (const childId of childrenOf.get(id) ?? []) visit(childId);
  };

  for (const [, roots] of sessionsOrdered) {
    roots.sort((a, b) => a.createdAtSeq - b.createdAtSeq || a.tapeName.localeCompare(b.tapeName));
    for (const root of roots) visit(root.id);
  }

  const width = PADDING * 2 + (maxDepth + 1) * COL_STRIDE - COL_GAP;
  const height = PADDING * 2 + row * ROW_STRIDE - ROW_GAP;

  return { nodes: positioned, edges, width, height };
}

/** Smallest `createdAtSeq` in the subtree rooted at `id`. */
function firstForkSeqUnder(
  id: string,
  childrenOf: Map<string, string[]>,
  nodeById: Map<string, TapeNode>,
): number {
  let best = nodeById.get(id)?.createdAtSeq ?? 0;
  if (best === 0) best = Number.POSITIVE_INFINITY;
  for (const childId of childrenOf.get(id) ?? []) {
    const sub = firstForkSeqUnder(childId, childrenOf, nodeById);
    if (sub < best) best = sub;
  }
  return best === Number.POSITIVE_INFINITY ? 0 : best;
}
