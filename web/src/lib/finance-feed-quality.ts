/*
 * Copyright 2026 Rararulab
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

interface TaggedFeedSource {
  tags: string[];
}

export function financeFeedQualityDisclosures(sources: TaggedFeedSource[]): string[] {
  if (sources.length === 0) return [];

  const disclosures: string[] = [];
  if (sources.every((source) => source.tags.includes('no-auth'))) {
    disclosures.push('No key required');
  }
  if (sources.some((source) => source.tags.includes('best-effort'))) {
    disclosures.push('Best effort');
  }
  if (
    sources.some(
      (source) => source.tags.includes('delayed') && source.tags.includes('unofficial-api'),
    )
  ) {
    disclosures.push('Delayed / unofficial');
  }
  if (sources.some((source) => source.tags.includes('region-dependent'))) {
    disclosures.push('Region dependent');
  }
  return disclosures;
}
