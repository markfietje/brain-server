/**
 * `sanitizeForBlock` parity with the Rust core's `fence::strip_markdown_refs`.
 *
 * ## WHY THIS SUITE EXISTS
 *
 * Both strips delete a markdown ref's URL and keep its label. A SINGLE pass
 * therefore heals a construct out of the surrounding prose: the label of an
 * outer link is re-emitted UNSCANNED, so
 *
 *     [![a](https://inner) c](https://outer)
 *
 * welds the label onto the outer URL into a LIVE `![a c](https://outer)` image
 * — a remote-image ref the strip was supposed to remove, pointing wherever the
 * author chose. That is a data-egress primitive aimed at whatever renders the
 * block next.
 *
 * The Rust side runs the strip to a FIXED POINT for exactly this reason
 * (`src/fence.rs`, `FIXPOINT_PASSES`, with the second pass pinned). This mirror
 * ran a single pass, so the plugin and the server disagreed about identical
 * bytes while the module doc claimed parity.
 *
 * It was not live only because the server's `sanitize_read` runs the correct
 * version on every recall hit before the plugin sees it. That is defence in
 * depth, not a reason to leave the mirror wrong: it becomes live the moment any
 * hit bypasses that seam (the byte-faithful export lanes) or the pinned origin
 * is served by a non-Rust implementation.
 *
 * Red-first: reverting `format.ts` step D to a single pass reds
 * `a_single_pass_would_heal_a_nested_ref_into_a_live_image`.
 */

import { describe, expect, test } from "vitest";
import { sanitizeForBlock } from "../src/format.js";

/** The welded form the fixed point is FOR: a live image ref that survived. */
const LIVE_REF = /!?\[[^\]]*\]\([^)]*\)/;

describe("sanitizeForBlock — markdown-ref parity with the Rust core", () => {
  test("strips a plain remote image ref and keeps its alt text", () => {
    expect(sanitizeForBlock("before ![alt](https://evil.example/p.png) after")).toBe(
      "before [alt] after",
    );
  });

  test("strips a plain link ref but keeps its label", () => {
    expect(sanitizeForBlock("see [the doc](https://example.com/a?b=c) now")).toBe(
      "see the doc now",
    );
  });

  test("a_single_pass_would_heal_a_nested_ref_into_a_live_image", () => {
    const plant = "x [![a](https://inner.example/i.png) c](https://outer.example/leak.png) y";
    const out = sanitizeForBlock(plant);

    expect(out).not.toContain("outer.example");
    expect(out).not.toContain("inner.example");
    // No ref syntax of either kind survives — the shape the Rust strip emits.
    expect(out).not.toMatch(LIVE_REF);
  });

  test("survives a deeply nested plant with no ref left behind", () => {
    // Each layer re-emits the inner label unscanned, so nesting DEPTH is
    // exactly the input a fixed point is for.
    const plant =
      "[![a](https://inner.example/i.png) b]".repeat(20) + "(https://outer.example/leak.png)";
    const out = sanitizeForBlock(plant);
    expect(out).not.toContain("http");
    expect(out).not.toMatch(LIVE_REF);
  });

  test("leaves plain prose with bare brackets and parens untouched", () => {
    expect(sanitizeForBlock("a plain sentence with [brackets] and (parens)")).toBe(
      "a plain sentence with [brackets] and (parens)",
    );
  });

  test("does not weld a live ref around a deleted hostile element", () => {
    // The ref strip ran BEFORE the hostile-element strip, so deleting a tag
    // between a label and its `(url)` welded the two into a live remote
    // image (audit N5). Both shapes are the SAME primitive as the nested
    // heal above, reached through a different deletion.
    for (const plant of [
      "![x]<script></script>(https://attacker.example/p.png)",
      "![x]<img src=x>(https://attacker.example/p.png)",
    ]) {
      const out = sanitizeForBlock(plant);
      expect(LIVE_REF.test(out), `welded a live ref: ${out}`).toBe(false);
    }
  });
});
