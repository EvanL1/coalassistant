// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import { claimLocalData, clearLocalAppData, getLocalOwner } from "./localAccount";

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

function seed() {
  localStorage.setItem("doudou_blend.coal_prefs.v1", '{"甲":{"enabled":true}}');
  localStorage.setItem("doudou_blend.history.v1", "[]");
  localStorage.setItem("doudou_blend.penalty_template.v1", '{"clauses":[]}');
  localStorage.setItem("别的应用", "留着");
  sessionStorage.setItem("doudou_blend.quote_draft.v1", "{}");
}

describe("本机数据归属", () => {
  it("没有归属记录的旧数据归第一个登录的账号", () => {
    seed();
    expect(getLocalOwner()).toBeNull();
    claimLocalData(" 张三 ");
    expect(getLocalOwner()).toBe("张三");
    expect(localStorage.getItem("doudou_blend.coal_prefs.v1")).not.toBeNull();
  });

  it("同一个账号再登录不动本机数据", () => {
    claimLocalData("张三");
    seed();
    claimLocalData("张三");
    expect(localStorage.getItem("doudou_blend.coal_prefs.v1")).not.toBeNull();
    expect(sessionStorage.getItem("doudou_blend.quote_draft.v1")).toBe("{}");
  });

  it("换账号登录丢掉上一个账号的全部本机数据, 包括扣款模板", () => {
    claimLocalData("张三");
    seed();
    claimLocalData("李四");
    expect(getLocalOwner()).toBe("李四");
    expect(localStorage.getItem("doudou_blend.coal_prefs.v1")).toBeNull();
    expect(localStorage.getItem("doudou_blend.history.v1")).toBeNull();
    expect(localStorage.getItem("doudou_blend.penalty_template.v1")).toBeNull();
    expect(sessionStorage.getItem("doudou_blend.quote_draft.v1")).toBeNull();
    expect(localStorage.getItem("别的应用")).toBe("留着");
  });

  it("空账号名不认领", () => {
    seed();
    claimLocalData("  ");
    expect(getLocalOwner()).toBeNull();
  });

  it("登出清掉云端有副本的数据与草稿, 留下账号名、扣款模板和还没导入云端的历史", () => {
    claimLocalData("张三");
    seed();
    clearLocalAppData();
    expect(localStorage.getItem("doudou_blend.coal_prefs.v1")).toBeNull();
    // 历史导入成功后才由 clearHistory 清; 登出时还没导入, 必须留着, 一退就永久丢了.
    expect(localStorage.getItem("doudou_blend.history.v1")).toBe("[]");
    expect(sessionStorage.getItem("doudou_blend.quote_draft.v1")).toBeNull();
    expect(localStorage.getItem("doudou_blend.penalty_template.v1")).toBe('{"clauses":[]}');
    expect(getLocalOwner()).toBe("张三");
    expect(localStorage.getItem("别的应用")).toBe("留着");
  });
});
