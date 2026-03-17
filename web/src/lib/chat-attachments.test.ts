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

import { describe, expect, it } from "vitest";

import {
  buildOutboundChatContent,
  fileToImageBlock,
  imageBlockSrc,
} from "./chat-attachments";

describe("chat attachments", () => {
  it("builds a multimodal payload when urls or inline images exist", () => {
    expect(
      buildOutboundChatContent("look", [
        { type: "image_url", url: "https://example.com/cat.png" },
      ]),
    ).toEqual([
      { type: "text", text: "look" },
      { type: "image_url", url: "https://example.com/cat.png" },
    ]);
  });

  it("renders image_base64 blocks as data urls", () => {
    expect(
      imageBlockSrc({
        type: "image_base64",
        media_type: "image/png",
        data: "AAAA",
      }),
    ).toBe("data:image/png;base64,AAAA");
  });

  it("turns a local file into an image_base64 block", async () => {
    await expect(
      fileToImageBlock(new File(["hi"], "tiny.png", { type: "image/png" })),
    ).resolves.toEqual({
      type: "image_base64",
      media_type: "image/png",
      data: "aGk=",
    });
  });
});
