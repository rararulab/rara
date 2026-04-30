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

import { ChevronDown, ChevronRight, GitFork } from 'lucide-react';
import { useMemo, useState } from 'react';

import type { TopologyEventEntry } from '@/hooks/use-topology-subscription';

import {
  buildTapeForest,
  layoutTapeForest,
  NODE_HEIGHT,
  NODE_WIDTH,
  type PositionedTapeNode,
} from './tape-tree-layout';

export interface TapeLineageViewProps {
  /** Every observed event from the topology subscription. */
  events: TopologyEventEntry[];
  /**
   * Currently focused session in the main timeline. Nodes whose
   * `sessionKey` matches are highlighted to keep the panel and
   * timeline visually linked.
   */
  activeSessionKey: string | null;
}

/**
 * Collapsible "Tape lineage" panel — renders the parent → child fork
 * forest derived from the topology event buffer (task #7 of #1999).
 *
 * Default-collapsed because most sessions never fork; expanding the
 * panel by default would dump an empty SVG on the operator. When
 * expanded with no forks, renders an explicit empty state instead of
 * a 0×0 SVG.
 */
export function TapeLineageView({ events, activeSessionKey }: TapeLineageViewProps) {
  const [open, setOpen] = useState(false);

  const layout = useMemo(() => layoutTapeForest(buildTapeForest(events)), [events]);

  return (
    <div className="rounded border border-border">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-xs font-medium text-muted-foreground hover:text-foreground"
        aria-expanded={open}
      >
        {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        <GitFork className="h-3 w-3" />
        <span>Tape lineage</span>
        <span className="ml-auto font-mono text-[10px] text-muted-foreground/70">
          {layout.nodes.length} {layout.nodes.length === 1 ? 'tape' : 'tapes'}
        </span>
      </button>

      {open && (
        <div className="border-t border-border p-2">
          {layout.nodes.length === 0 ? (
            <div className="px-1 py-2 text-[11px] text-muted-foreground">
              No forks yet — this session has not anchored a child tape.
            </div>
          ) : (
            <TapeForestSvg layout={layout} activeSessionKey={activeSessionKey} />
          )}
        </div>
      )}
    </div>
  );
}

interface TapeForestSvgProps {
  layout: ReturnType<typeof layoutTapeForest>;
  activeSessionKey: string | null;
}

/**
 * Render the laid-out forest as a single SVG. Nodes are rounded rects
 * with the tape name; edges are L-shaped paths from the parent's right
 * edge to the child's left edge. Hover reveals the full tape metadata
 * via a native `<title>` tooltip — no portal or popover dep needed.
 */
function TapeForestSvg({ layout, activeSessionKey }: TapeForestSvgProps) {
  const nodeById = useMemo(
    () => new Map(layout.nodes.map((n) => [n.id, n] as const)),
    [layout.nodes],
  );

  return (
    <div className="overflow-auto">
      <svg
        width={layout.width}
        height={layout.height}
        role="img"
        aria-label="Tape fork lineage"
        className="text-foreground"
      >
        <g>
          {layout.edges.map((edge) => {
            const parent = nodeById.get(edge.parentId);
            const child = nodeById.get(edge.childId);
            if (!parent || !child) return null;
            return (
              <EdgePath
                key={`${edge.parentId}->${edge.childId}`}
                parent={parent}
                child={child}
                anchor={edge.forkedAtAnchor}
              />
            );
          })}
        </g>
        <g>
          {layout.nodes.map((node) => (
            <TapeNodeRect
              key={node.id}
              node={node}
              highlighted={activeSessionKey != null && node.sessionKey === activeSessionKey}
            />
          ))}
        </g>
      </svg>
    </div>
  );
}

function EdgePath({
  parent,
  child,
  anchor,
}: {
  parent: PositionedTapeNode;
  child: PositionedTapeNode;
  anchor: string | null;
}) {
  const startX = parent.x + NODE_WIDTH;
  const startY = parent.y + NODE_HEIGHT / 2;
  const endX = child.x;
  const endY = child.y + NODE_HEIGHT / 2;
  const midX = startX + (endX - startX) / 2;
  // Smooth S-curve so vertically-distant siblings still read as belonging
  // to the same parent — straight L-paths overlap at the elbow.
  const d = `M ${startX} ${startY} C ${midX} ${startY}, ${midX} ${endY}, ${endX} ${endY}`;
  return (
    <g>
      <path
        d={d}
        fill="none"
        stroke="currentColor"
        strokeOpacity={0.35}
        strokeWidth={1}
      />
      {anchor && (
        <text
          x={midX}
          y={(startY + endY) / 2 - 2}
          textAnchor="middle"
          className="fill-muted-foreground font-mono"
          fontSize={9}
        >
          @{truncate(anchor, 12)}
        </text>
      )}
    </g>
  );
}

function TapeNodeRect({
  node,
  highlighted,
}: {
  node: PositionedTapeNode;
  highlighted: boolean;
}) {
  const tooltip = [
    `tape: ${node.tapeName}`,
    `session: ${node.sessionKey}`,
    node.forkedAtAnchor ? `forked@${node.forkedAtAnchor}` : 'root tape',
    node.createdAtSeq > 0 ? `seq #${node.createdAtSeq}` : null,
  ]
    .filter(Boolean)
    .join('\n');

  return (
    <g>
      <title>{tooltip}</title>
      <rect
        x={node.x}
        y={node.y}
        width={NODE_WIDTH}
        height={NODE_HEIGHT}
        rx={4}
        ry={4}
        className={
          highlighted
            ? 'fill-primary/15 stroke-primary'
            : 'fill-muted/40 stroke-border'
        }
        strokeWidth={1}
      />
      <text
        x={node.x + 8}
        y={node.y + NODE_HEIGHT / 2 + 4}
        className="fill-foreground font-mono"
        fontSize={11}
      >
        {truncate(node.tapeName, 22)}
      </text>
    </g>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
