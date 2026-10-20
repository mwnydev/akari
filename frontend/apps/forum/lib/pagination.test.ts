import { describe, expect, it } from "vitest";
import { floorPage, hotRequest, pageWindow, parsePositiveInt } from "./pagination";

describe("floorPage", () => {
  it("maps floors to 20-floor pages", () => {
    expect(floorPage(1)).toBe(1);
    expect(floorPage(20)).toBe(1);
    expect(floorPage(21)).toBe(2);
    expect(floorPage(4000)).toBe(200);
    expect(floorPage(0)).toBe(1);
  });
});

describe("pageWindow", () => {
  it("shows every page when there are few", () => {
    expect(pageWindow(1, 1)).toEqual([1]);
    expect(pageWindow(2, 5)).toEqual([1, 2, 3, 4, 5]);
  });

  it("collapses distant pages and fills single-page gaps", () => {
    expect(pageWindow(1, 7)).toEqual([1, 2, 3, "gap", 7]);
    expect(pageWindow(5, 7)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(pageWindow(50, 100)).toEqual([1, "gap", 48, 49, 50, 51, 52, "gap", 100]);
    expect(pageWindow(100, 100)).toEqual([1, "gap", 98, 99, 100]);
  });
});

describe("parsePositiveInt", () => {
  it("falls back to page 1 on bad input", () => {
    expect(parsePositiveInt(undefined)).toBe(1);
    expect(parsePositiveInt("0")).toBe(1);
    expect(parsePositiveInt("-3")).toBe(1);
    expect(parsePositiveInt("abc")).toBe(1);
    expect(parsePositiveInt(["4", "5"])).toBe(4);
    expect(parsePositiveInt("12")).toBe(12);
  });
});

describe("hotRequest", () => {
  it("only asks for the hot list when its tab is open or paged", () => {
    expect(hotRequest({})).toEqual({ active: false, cursor: undefined });
    expect(hotRequest({ tab: "latest", page: "2" })).toEqual({ active: false, cursor: undefined });
    expect(hotRequest({ tab: "hot" })).toEqual({ active: true, cursor: undefined });
    expect(hotRequest({ hot: "1.5_9" })).toEqual({ active: true, cursor: "1.5_9" });
    expect(hotRequest({ hot: "" })).toEqual({ active: false, cursor: undefined });
    expect(hotRequest({ hot: ["a", "b"] })).toEqual({ active: false, cursor: undefined });
  });
});
