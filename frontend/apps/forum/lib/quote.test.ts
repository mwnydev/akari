import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";
import { ownText, QUOTE_CHARS } from "./quote";

const body = (html: string) => {
  const doc = new JSDOM(`<div id="b">${html}</div>`).window.document;
  return doc.getElementById("b") as HTMLElement;
};

describe("ownText", () => {
  it("drops the quotes inside the quoted post", () => {
    const b = body(
      '<blockquote><p><a href="/t/1#floor-2">#2 张三</a>：<blockquote><p>#1 李四：原文</p></blockquote>二楼</p></blockquote>' +
        "<p>三楼自己的话</p><p>第二段</p>",
    );
    expect(ownText(b)).toBe("三楼自己的话第二段");
  });

  it("collapses whitespace and caps the length by characters", () => {
    expect(ownText(body("<p> a \n b </p>"))).toBe("a b");
    const long = ownText(body(`<p>${"😀".repeat(QUOTE_CHARS + 5)}</p>`));
    expect([...long]).toHaveLength(QUOTE_CHARS + 1);
    expect(long.endsWith("…")).toBe(true);
  });

  it("works on a selection fragment", () => {
    const b = body("<p>前<blockquote>引</blockquote>后</p>");
    const range = b.ownerDocument.createRange();
    range.selectNodeContents(b);
    expect(ownText(range.cloneContents())).toBe("前后");
  });
});
