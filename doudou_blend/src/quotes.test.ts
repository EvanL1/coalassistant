import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { archiveQuotes, fetchLatestQuotes, type QuoteItemInput } from "./quotes";

const items: QuoteItemInput[] = [{ name: "甲", fob: 2150, frt: 80, props: { S: 0.6 }, changed: [] }];
const archived = { name: "甲", fob: 2150, estimate: 2000, gap: 150, ratio: 1.075, benchmark: "基准" };
const latest = { name: "甲", fob: 2150, frt: 80, quoted_on: "2026-10-28" };
const reply = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
beforeEach(() => vi.stubGlobal("fetch", vi.fn()));
afterEach(() => vi.unstubAllGlobals());

it("按原样提交询价批次并返回服务端对比", async () => {
  vi.mocked(fetch).mockResolvedValue(reply({ ok: true, items: [archived] }));
  expect(await archiveQuotes("batch-1", items)).toEqual([archived]);
  expect(fetch).toHaveBeenCalledWith("/api/quotes", {
    method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ batch_id: "batch-1", items }),
  });
});

it("存档失败保留服务端原因", async () => {
  vi.mocked(fetch).mockResolvedValue(reply({ ok: false, reason: "数据库尚未配置" }, 503));
  await expect(archiveQuotes("batch-1", items)).rejects.toThrow("数据库尚未配置");
});

it.each([null, {}, { ok: true, items: {} }, { ok: true, items: [null] }, { ok: true, items: [{ ...archived, gap: "坏值" }] }])("存档响应损坏时抛错: %j", async (body) => {
  vi.mocked(fetch).mockResolvedValue(reply(body));
  await expect(archiveQuotes("batch-1", items)).rejects.toThrow();
});

it("最近询价按煤名返回, 只从同源读取且不缓存", async () => {
  vi.mocked(fetch).mockResolvedValue(reply({ ok: true, quotes: [latest] }));
  expect(await fetchLatestQuotes()).toEqual(new Map([["甲", latest]]));
  expect(fetch).toHaveBeenCalledWith("/api/quotes/latest", { credentials: "same-origin", cache: "no-store" });
});

it.each([null, {}, { ok: true, quotes: {} }, { ok: true, quotes: [null] }, { ok: true, quotes: [{ ...latest, fob: "2150" }] }])("最近询价损坏时返回空 Map: %j", async (body) => {
  vi.mocked(fetch).mockResolvedValue(reply(body));
  expect(await fetchLatestQuotes()).toEqual(new Map());
});

it("最近询价 HTTP 或网络失败不阻塞页面", async () => {
  vi.mocked(fetch).mockResolvedValueOnce(reply({}, 503)).mockRejectedValueOnce(new Error("断网"));
  expect(await fetchLatestQuotes()).toEqual(new Map());
  expect(await fetchLatestQuotes()).toEqual(new Map());
});
