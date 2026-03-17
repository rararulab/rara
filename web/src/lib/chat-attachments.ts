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

import type { ChatContentBlock } from "@/api/types";

export type ImageChatContentBlock = Extract<
  ChatContentBlock,
  { type: "image_url" | "image_base64" }
>;

export function buildOutboundChatContent(
  text: string,
  blocks: ChatContentBlock[],
): string | ChatContentBlock[] {
  const trimmed = text.trim();

  if (blocks.length === 0) {
    return trimmed;
  }

  return [
    ...(trimmed ? [{ type: "text", text: trimmed } satisfies ChatContentBlock] : []),
    ...blocks,
  ];
}

export function imageBlockSrc(block: ImageChatContentBlock): string {
  return block.type === "image_url"
    ? block.url
    : `data:${block.media_type};base64,${block.data}`;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";

  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }

  return btoa(binary);
}

export async function fileToImageBlock(file: File): Promise<ImageChatContentBlock> {
  const bytes = new Uint8Array(await file.arrayBuffer());

  return {
    type: "image_base64",
    media_type: file.type || "application/octet-stream",
    data: bytesToBase64(bytes),
  };
}
