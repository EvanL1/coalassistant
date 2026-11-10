// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import type { BlendResult } from "../types";
import { clearQuoteDraft, loadQuoteDraft, saveQuoteDraft, type QuoteDraft } from "./quoteDraft";

const todayResult = {
  ok: true, recipe: {}, cost: null, orders: [], indicator_check: [], warnings: [],
} as unknown as BlendResult;

beforeEach(() => sessionStorage.clear());

describe("询价草稿", () => {
  it("存进去读出来, 清掉就没了", () => {
    const draft: QuoteDraft = {
      todayResult,
      batchId: "批次",
      rows: [{ name: "甲", fob: "2,050", frt: "", props: { S: "0.6" } }],
    };
    saveQuoteDraft(draft);
    expect(loadQuoteDraft()).toEqual(draft);
    clearQuoteDraft();
    expect(loadQuoteDraft()).toBeNull();
  });

  it("还没动过的草稿 rows 为 null", () => {
    saveQuoteDraft({ todayResult, batchId: "批次", rows: null });
    expect(loadQuoteDraft()?.rows).toBeNull();
  });

  it("只存在本标签页 (sessionStorage), 不进 localStorage", () => {
    localStorage.clear();
    saveQuoteDraft({ todayResult, batchId: "批次", rows: null });
    expect(localStorage.length).toBe(0);
  });

  it("坏数据当没有草稿", () => {
    for (const raw of [
      "{坏",
      "null",
      JSON.stringify({ todayResult, batchId: 1, rows: null }),
      JSON.stringify({ todayResult: { orders: "x" }, batchId: "b", rows: null }),
      JSON.stringify({ todayResult, batchId: "b", rows: [{ name: "甲", fob: 1, frt: "", props: {} }] }),
      JSON.stringify({ todayResult, batchId: "b", rows: [{ name: "甲", fob: "", frt: "", props: { S: 1 } }] }),
      JSON.stringify({ todayResult, batchId: "b", rows: "x" }),
    ]) {
      sessionStorage.setItem("doudou_blend.quote_draft.v1", raw);
      expect(loadQuoteDraft()).toBeNull();
    }
  });
});
