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

import { ChevronDown, ChevronRight } from 'lucide-react';
import { useState } from 'react';

/** Threshold beyond which string values are truncated. */
const STRING_TRUNCATE_LEN = 200;

/** Renders a string value with optional truncation for long strings. */
function StringValue({ value }: { value: string }) {
  const [expanded, setExpanded] = useState(false);
  const needsTruncation = value.length > STRING_TRUNCATE_LEN;

  const display =
    needsTruncation && !expanded ? value.slice(0, STRING_TRUNCATE_LEN) + '...' : value;

  return (
    <span className="text-green-400">
      &quot;{display}&quot;
      {needsTruncation && (
        <button
          className="ml-1 text-zinc-500 hover:text-zinc-300 underline text-[10px]"
          onClick={(e) => {
            e.stopPropagation();
            setExpanded(!expanded);
          }}
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </span>
  );
}

/** Renders a JSON object with collapsible keys. */
function ObjectNode({ data, depth }: { data: Record<string, unknown>; depth: number }) {
  const keys = Object.keys(data);
  const [expanded, setExpanded] = useState(depth < 2);

  if (keys.length === 0) {
    return <span className="text-zinc-500">{'{}'}</span>;
  }

  return (
    <span>
      <button
        className="inline-flex items-center text-zinc-500 hover:text-zinc-300"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? (
          <ChevronDown className="inline h-3 w-3" />
        ) : (
          <ChevronRight className="inline h-3 w-3" />
        )}
        <span className="text-zinc-500">{`{${keys.length}}`}</span>
      </button>
      {expanded && (
        <div className="ml-4 border-l border-zinc-800 pl-2">
          {keys.map((key) => (
            <div key={key} className="leading-relaxed">
              <span className="text-purple-400">{key}</span>
              <span className="text-zinc-500">: </span>
              <JsonTree data={data[key]} depth={depth + 1} />
            </div>
          ))}
        </div>
      )}
    </span>
  );
}

/** Renders a JSON array with collapsible items. */
function ArrayNode({ data, depth }: { data: unknown[]; depth: number }) {
  const [expanded, setExpanded] = useState(depth < 2);

  if (data.length === 0) {
    return <span className="text-zinc-500">{'[]'}</span>;
  }

  return (
    <span>
      <button
        className="inline-flex items-center text-zinc-500 hover:text-zinc-300"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? (
          <ChevronDown className="inline h-3 w-3" />
        ) : (
          <ChevronRight className="inline h-3 w-3" />
        )}
        <span className="text-zinc-500">{`[${data.length}]`}</span>
      </button>
      {expanded && (
        <div className="ml-4 border-l border-zinc-800 pl-2">
          {data.map((item, i) => (
            <div key={i} className="leading-relaxed">
              <span className="text-zinc-600 mr-1">{i}:</span>
              <JsonTree data={item} depth={depth + 1} />
            </div>
          ))}
        </div>
      )}
    </span>
  );
}

/** Recursive collapsible JSON tree renderer with dark-theme syntax coloring. */
export function JsonTree({ data, depth = 0 }: { data: unknown; depth?: number }) {
  if (data === null || data === undefined) {
    return <span className="text-zinc-500">null</span>;
  }

  if (typeof data === 'string') {
    return <StringValue value={data} />;
  }

  if (typeof data === 'number') {
    return <span className="text-cyan-400">{String(data)}</span>;
  }

  if (typeof data === 'boolean') {
    return <span className="text-amber-400">{String(data)}</span>;
  }

  if (Array.isArray(data)) {
    return <ArrayNode data={data} depth={depth} />;
  }

  if (typeof data === 'object') {
    return <ObjectNode data={data as Record<string, unknown>} depth={depth} />;
  }

  // Fallback for unexpected types
  return <span className="text-zinc-400">{String(data)}</span>;
}
